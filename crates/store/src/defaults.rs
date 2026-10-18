//! What an owner's new raffles start with (`owner_defaults`): who organizes
//! them and their additional rules. A raffle gets a copy as it is created.

use serde::Deserialize;

use crate::{Db, Result, now};

/// An owner's defaults, already validated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Defaults {
    pub organizer_name: String,
    pub contact_email: String,
    pub rules: String,
}

/// `owner`'s defaults, if they saved any.
pub async fn get(db: &Db, owner: &str) -> Result<Option<Defaults>> {
    db.query("SELECT organizer_name, contact_email, rules FROM owner_defaults WHERE owner_id = ?")
        .bind(owner)
        .optional()
        .await
}

/// Saves `owner`'s defaults, replacing any they had.
pub async fn save(db: &Db, owner: &str, d: &Defaults) -> Result<()> {
    db.query(
        "INSERT INTO owner_defaults (owner_id, organizer_name, contact_email, rules, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (owner_id) DO UPDATE SET organizer_name = excluded.organizer_name,
             contact_email = excluded.contact_email, rules = excluded.rules,
             updated_at = excluded.updated_at",
    )
    .bind(owner)
    .bind(&d.organizer_name)
    .bind(&d.contact_email)
    .bind(&d.rules)
    .bind(now())
    .run()
    .await
    .map(|_| ())
}
