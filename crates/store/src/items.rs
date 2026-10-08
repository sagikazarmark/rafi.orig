//! A raffle's prizes.

use serde::Deserialize;

use crate::{Db, Result};

/// A row of `items`.
#[derive(Debug, Clone, Deserialize)]
pub struct ItemRow {
    pub id: String,
    pub raffle_id: String,
    pub name: String,
    pub sponsor: String,
    pub description: String,
    pub qty: i64,
    pub position: i64,
}

/// An item's editable fields, already validated.
#[derive(Debug, Clone)]
pub struct ItemFields<'a> {
    pub name: &'a str,
    pub sponsor: &'a str,
    pub description: &'a str,
    pub qty: u32,
    pub position: i64,
}

/// The order items are listed and drawn in.
pub(crate) const ORDER: &str = "ORDER BY position, id";

/// A raffle's items, in draw order.
pub async fn list(db: &Db, raffle_id: &str) -> Result<Vec<ItemRow>> {
    db.query(format!("SELECT * FROM items WHERE raffle_id = ? {ORDER}"))
        .bind(raffle_id)
        .all()
        .await
}

pub async fn insert(db: &Db, id: &str, raffle_id: &str, item: &ItemFields<'_>) -> Result<()> {
    db.query(
        "INSERT INTO items (id, raffle_id, name, sponsor, description, qty, position)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(raffle_id)
    .bind(item.name)
    .bind(item.sponsor)
    .bind(item.description)
    .bind(item.qty)
    .bind(item.position)
    .run()
    .await
    .map(|_| ())
}

/// Returns `false` if the raffle has no such item.
pub async fn update(db: &Db, id: &str, raffle_id: &str, item: &ItemFields<'_>) -> Result<bool> {
    let changed = db
        .query(
            "UPDATE items SET name = ?, sponsor = ?, description = ?, qty = ?, position = ?
             WHERE id = ? AND raffle_id = ?",
        )
        .bind(item.name)
        .bind(item.sponsor)
        .bind(item.description)
        .bind(item.qty)
        .bind(item.position)
        .bind(id)
        .bind(raffle_id)
        .run()
        .await?;
    Ok(changed > 0)
}

pub async fn delete(db: &Db, id: &str, raffle_id: &str) -> Result<()> {
    db.query("DELETE FROM items WHERE id = ? AND raffle_id = ?")
        .bind(id)
        .bind(raffle_id)
        .run()
        .await
        .map(|_| ())
}
