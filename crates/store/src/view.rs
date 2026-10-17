//! Reading what the projector and the admin see: the raffle with all its
//! prizes and entrants, and the raffle as drawn, in one snapshot.

use rafi_raffle::Raffle;

use crate::{
    Db, Result, Row,
    entrants::{self, EntrantRow},
    prizes::{self, PrizeRow},
    raffles::{self, RaffleRow},
};

/// A raffle, its prizes (in draw order), its entrants (in the order they
/// entered) and the raffle as drawn, read in one [`Db::batch_rows`], so the
/// raffle's version and the content agree.
#[derive(Debug, Clone)]
pub struct Full {
    pub raffle: RaffleRow,
    pub prizes: Vec<PrizeRow>,
    pub entrants: Vec<EntrantRow>,
    /// The raffle as drawn, once the engine has opened it
    /// ([`raffles::drawn`]).
    pub drawn: Option<Raffle>,
}

impl Full {
    /// Reads the raffle, or `None` if it doesn't exist.
    pub async fn read(db: &Db, raffle_id: &str) -> Result<Option<Self>> {
        let mut sets = db
            .batch_rows(vec![
                db.query("SELECT * FROM raffles WHERE id = ?")
                    .bind(raffle_id),
                db.query(format!(
                    "SELECT * FROM prizes WHERE raffle_id = ? {}",
                    prizes::ORDER
                ))
                .bind(raffle_id),
                db.query(format!(
                    "SELECT * FROM entrants WHERE raffle_id = ? {}",
                    entrants::ORDER
                ))
                .bind(raffle_id),
                raffles::events_query(db, raffle_id),
            ])
            .await?
            .into_iter();
        let (Some(raffle), Some(prizes), Some(entrants), Some(events)) =
            (sets.next(), sets.next(), sets.next(), sets.next())
        else {
            return Ok(None);
        };
        let Some(raffle) = raffle.into_iter().next() else {
            return Ok(None);
        };
        let raffle: RaffleRow = raffle.decode()?;
        let drawn = raffles::rebuild(raffle.opened_at, raffle.manifest.as_deref(), events)?;
        Ok(Some(Self {
            raffle,
            prizes: decode_all(prizes)?,
            entrants: decode_all(entrants)?,
            drawn,
        }))
    }
}

fn decode_all<T: serde::de::DeserializeOwned>(rows: Vec<Row>) -> Result<Vec<T>> {
    rows.into_iter().map(Row::decode).collect()
}
