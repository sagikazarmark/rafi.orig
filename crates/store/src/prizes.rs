//! A raffle's prizes.

use serde::Deserialize;

use crate::{Db, Result};

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

pub async fn insert(db: &Db, id: &str, raffle_id: &str, prize: &PrizeFields<'_>) -> Result<()> {
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
    .run()
    .await
    .map(|_| ())
}

/// Returns `false` if the raffle has no such prize.
pub async fn update(db: &Db, id: &str, raffle_id: &str, prize: &PrizeFields<'_>) -> Result<bool> {
    let changed = db
        .query(
            "UPDATE prizes SET name = ?, sponsor = ?, description = ?, qty = ?, position = ?
             WHERE id = ? AND raffle_id = ?",
        )
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

pub async fn delete(db: &Db, id: &str, raffle_id: &str) -> Result<()> {
    db.query("DELETE FROM prizes WHERE id = ? AND raffle_id = ?")
        .bind(id)
        .bind(raffle_id)
        .run()
        .await
        .map(|_| ())
}
