//! Reading what the projector and the admin see: the raffle with all its
//! items and entrants, in one snapshot.

use crate::{
    Db, Result, Row,
    entrants::{self, EntrantRow},
    items::{self, ItemRow},
    raffles::RaffleRow,
};

/// A raffle, its items (in draw order) and its entrants (in the order they
/// entered), read in one [`Db::batch_rows`], so the raffle's version and the
/// content agree.
#[derive(Debug, Clone)]
pub struct Full {
    pub raffle: RaffleRow,
    pub items: Vec<ItemRow>,
    pub entrants: Vec<EntrantRow>,
}

impl Full {
    /// Reads the raffle, or `None` if it doesn't exist.
    pub async fn read(db: &Db, raffle_id: &str) -> Result<Option<Self>> {
        let mut sets = db
            .batch_rows(vec![
                db.query("SELECT * FROM raffles WHERE id = ?")
                    .bind(raffle_id),
                db.query(format!(
                    "SELECT * FROM items WHERE raffle_id = ? {}",
                    items::ORDER
                ))
                .bind(raffle_id),
                db.query(format!(
                    "SELECT * FROM entrants WHERE raffle_id = ? {}",
                    entrants::ORDER
                ))
                .bind(raffle_id),
            ])
            .await?
            .into_iter();
        let (Some(raffle), Some(items), Some(entrants)) = (sets.next(), sets.next(), sets.next())
        else {
            return Ok(None);
        };
        let Some(raffle) = raffle.into_iter().next() else {
            return Ok(None);
        };
        Ok(Some(Self {
            raffle: raffle.decode()?,
            items: decode_all(items)?,
            entrants: decode_all(entrants)?,
        }))
    }
}

fn decode_all<T: serde::de::DeserializeOwned>(rows: Vec<Row>) -> Result<Vec<T>> {
    rows.into_iter().map(Row::decode).collect()
}
