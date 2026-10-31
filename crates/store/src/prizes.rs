//! A raffle's prizes.

use std::collections::HashMap;

use rafi_lifecycle::{RaffleState, prize::Delivery};
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
    pub delivery: Delivery,
}

/// A prize's editable fields, already validated.
#[derive(Debug, Clone)]
pub struct PrizeFields<'a> {
    pub name: &'a str,
    pub sponsor: &'a str,
    pub description: &'a str,
    pub qty: u32,
    pub position: i64,
    pub delivery: Delivery,
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

/// How each of a raffle's prizes reaches its winner, by prize ID.
pub async fn deliveries(db: &Db, raffle_id: &str) -> Result<HashMap<String, Delivery>> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        delivery: Delivery,
    }
    let rows: Vec<Row> = db
        .query("SELECT id, delivery FROM prizes WHERE raffle_id = ?")
        .bind(raffle_id)
        .all()
        .await?;
    Ok(rows.into_iter().map(|r| (r.id, r.delivery)).collect())
}

/// Adds a prize, only while the raffle's prizes can change. Returns `false`
/// if they can't.
pub async fn insert(db: &Db, id: &str, raffle_id: &str, prize: &PrizeFields<'_>) -> Result<bool> {
    let inserted = db
        .query(format!(
            "INSERT INTO prizes (id, raffle_id, name, sponsor, description, qty, position, delivery)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8
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
        .bind(prize.delivery.as_str())
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
        "INSERT INTO prizes (id, raffle_id, name, sponsor, description, qty, position, delivery)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(raffle_id)
    .bind(prize.name)
    .bind(prize.sponsor)
    .bind(prize.description)
    .bind(prize.qty)
    .bind(prize.position)
    .bind(prize.delivery.as_str())
}

/// Changes a prize, only while the raffle's prizes can change. Returns
/// `false` if nothing changed: no such prize, or they can't change.
pub async fn update(db: &Db, id: &str, raffle_id: &str, prize: &PrizeFields<'_>) -> Result<bool> {
    let changed = db
        .query(format!(
            "UPDATE prizes SET name = ?1, sponsor = ?2, description = ?3, qty = ?4, position = ?5,
                               delivery = ?6
             WHERE id = ?7 AND raffle_id = ?8 AND {}",
            editable(8)
        ))
        .bind(prize.name)
        .bind(prize.sponsor)
        .bind(prize.description)
        .bind(prize.qty)
        .bind(prize.position)
        .bind(prize.delivery.as_str())
        .bind(id)
        .bind(raffle_id)
        .run()
        .await?;
    Ok(changed > 0)
}

/// Changes how a prize reaches its winner, only while that can change
/// ([`RaffleState::prize_delivery_editable`]): after the freeze too, as it
/// isn't in the manifest. Returns `false` if nothing changed: no such prize,
/// or the raffle is over.
pub async fn set_delivery(db: &Db, id: &str, raffle_id: &str, delivery: Delivery) -> Result<bool> {
    let changed = db
        .query(format!(
            "UPDATE prizes SET delivery = ?1
             WHERE id = ?2 AND raffle_id = ?3
               AND EXISTS (SELECT 1 FROM raffles WHERE id = ?3 AND state IN ({}))",
            crate::raffles::states(RaffleState::prize_delivery_editable)
        ))
        .bind(delivery.as_str())
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
