//! What an owner's new raffles start with (`owner_defaults`): who organizes
//! them, their privacy contact and text, their additional rules and their
//! retention period. A raffle gets a copy as it is created.

use serde::Deserialize;

use crate::{Db, Result};

/// An owner's defaults, already validated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Defaults {
    pub organizer_name: String,
    pub contact_email: String,
    pub privacy_email: String,
    pub privacy_text: String,
    pub privacy_url: String,
    pub rules: String,
    /// `None`: the installation's default.
    pub retention_days: Option<i64>,
}

/// `owner`'s defaults, if they saved any.
pub async fn get(db: &Db, owner: &str) -> Result<Option<Defaults>> {
    db.query(
        "SELECT organizer_name, contact_email, privacy_email, privacy_text, privacy_url, rules, retention_days
         FROM owner_defaults WHERE owner_id = ?",
    )
    .bind(owner)
    .optional()
    .await
}

/// Saves `owner`'s defaults at `now`, replacing any they had.
pub async fn save(db: &Db, owner: &str, d: &Defaults, now: i64) -> Result<()> {
    db.query(
        "INSERT INTO owner_defaults (owner_id, organizer_name, contact_email, privacy_email, privacy_text,
                                     privacy_url, rules, retention_days, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (owner_id) DO UPDATE SET organizer_name = excluded.organizer_name,
             contact_email = excluded.contact_email, privacy_email = excluded.privacy_email,
             privacy_text = excluded.privacy_text, privacy_url = excluded.privacy_url,
             rules = excluded.rules, retention_days = excluded.retention_days,
             updated_at = excluded.updated_at",
    )
    .bind(owner)
    .bind(&d.organizer_name)
    .bind(&d.contact_email)
    .bind(&d.privacy_email)
    .bind(&d.privacy_text)
    .bind(&d.privacy_url)
    .bind(&d.rules)
    .bind(d.retention_days)
    .bind(now)
    .run()
    .await
    .map(|_| ())
}

/// Forgets `owner`'s defaults: they are leaving.
pub async fn delete(db: &Db, owner: &str) -> Result<()> {
    db.query("DELETE FROM owner_defaults WHERE owner_id = ?")
        .bind(owner)
        .run()
        .await
        .map(|_| ())
}
