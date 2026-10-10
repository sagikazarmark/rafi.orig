//! A raffle's prizes.

use rafi_lifecycle::RaffleState;
use serde::Deserialize;

use crate::{Db, Query, Result};

/// The SQL condition that raffle `?{param}`'s prizes can still change
/// ([`RaffleState::prizes_editable`]): writes check it in the same statement,
/// so none lands after the freeze.
fn editable(param: u8) -> String {
    format!(
        "EXISTS (SELECT 1 FROM raffles WHERE id = ?{param} AND state IN ({}))",
        crate::raffles::states(RaffleState::prizes_editable)
    )
}

/// A row of `prizes`.
#[derive(Debug, Clone, Deserialize)]
pub struct PrizeRow {
    pub id: String,
    pub raffle_id: String,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: i64,
    pub position: i64,
}

/// A prize's editable fields, already validated.
#[derive(Debug, Clone)]
pub struct PrizeFields<'a> {
    pub name: &'a str,
    pub sponsor: &'a str,
    pub description: &'a str,
    pub qty: u32,
    pub position: i64,
}

/// The order prizes are listed and drawn in.
pub(crate) const ORDER: &str = "ORDER BY position, id";

/// A raffle's prizes, in draw order.
pub async fn list(db: &Db, raffle_id: &str) -> Result<Vec<PrizeRow>> {
    db.query(format!("SELECT * FROM prizes WHERE raffle_id = ? {ORDER}"))
        .bind(raffle_id)
        .all()
        .await
}

/// Adds a prize, only while the raffle's prizes can change. Returns `false`
/// if they can't.
pub async fn insert(db: &Db, id: &str, raffle_id: &str, prize: &PrizeFields<'_>) -> Result<bool> {
    let inserted = db
        .query(format!(
            "INSERT INTO prizes (id, raffle_id, name, sponsor, description, qty, position)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7
             WHERE {}",
            editable(2)
        ))
        .bind(id)
        .bind(raffle_id)
        .bind(prize.name)
        .bind(prize.sponsor)
        .bind(prize.description)
        .bind(prize.qty)
        .bind(prize.position)
        .run()
        .await?;
    Ok(inserted > 0)
}

/// Inserts a prize unguarded: for a raffle written in the same batch.
pub(crate) fn insert_query<'a>(
    db: &'a Db,
    id: &str,
    raffle_id: &str,
    prize: &PrizeFields<'_>,
) -> Query<'a> {
    db.query(
        "INSERT INTO prizes (id, raffle_id, name, sponsor, description, qty, position)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(raffle_id)
    .bind(prize.name)
    .bind(prize.sponsor)
    .bind(prize.description)
    .bind(prize.qty)
    .bind(prize.position)
}

/// Changes a prize, only while the raffle's prizes can change. Returns
/// `false` if nothing changed: no such prize, or they can't change.
pub async fn update(db: &Db, id: &str, raffle_id: &str, prize: &PrizeFields<'_>) -> Result<bool> {
    let changed = db
        .query(format!(
            "UPDATE prizes SET name = ?1, sponsor = ?2, description = ?3, qty = ?4, position = ?5
             WHERE id = ?6 AND raffle_id = ?7 AND {}",
            editable(7)
        ))
        .bind(prize.name)
        .bind(prize.sponsor)
        .bind(prize.description)
        .bind(prize.qty)
        .bind(prize.position)
        .bind(id)
        .bind(raffle_id)
        .run()
        .await?;
    Ok(changed > 0)
}

/// Removes a prize, only while the raffle's prizes can change. Returns
/// `false` if nothing was removed: no such prize, or they can't change.
pub async fn delete(db: &Db, id: &str, raffle_id: &str) -> Result<bool> {
    let deleted = db
        .query(format!(
            "DELETE FROM prizes WHERE id = ?1 AND raffle_id = ?2 AND {}",
            editable(2)
        ))
        .bind(id)
        .bind(raffle_id)
        .run()
        .await?;
    Ok(deleted > 0)
}
