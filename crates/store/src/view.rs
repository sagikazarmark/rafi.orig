//! Reading what the projector and the admin see: the raffle with its prizes,
//! its entrants (all of them for the admin, [`Full`]; a summary for the
//! screens that poll, [`Lite`]) and the raffle as drawn, in one snapshot.

use rafi_lifecycle::entrant::Source;
use rafi_raffle::Raffle;
use serde::Deserialize;

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

/// How many names [`Summary::recent_names`] holds.
pub const RECENT_NAMES: usize = 20;

/// What the screens show of a raffle's entrants: how many, the latest names
/// and the drawn ones. The same from every entrant's row ([`Summary::of`],
/// for the admin, which reads them all) as from the database ([`Lite`], for
/// the screens, which poll).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// Every entrant.
    pub count: u64,
    /// The entrants whose personal data isn't erased.
    pub unerased: u64,
    /// The latest names that can be shown (neither hidden nor erased) of
    /// entrants in at or before the time asked for, newest first, at most
    /// [`RECENT_NAMES`].
    pub recent_names: Vec<String>,
    /// The entrants in a draw, by ID.
    pub drawn: Vec<DrawnEntrant>,
    /// Whether anyone came in with an add from a list (the list tab), not
    /// the one-person form or a join: their row number in it says so, and
    /// stays when they are erased. People on a list may never have heard of
    /// the raffle unless the organizer told them, which the projector and
    /// the freeze's reminder are about.
    pub list_added: bool,
}

/// An entrant in a draw, as the screens name them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DrawnEntrant {
    pub id: String,
    pub display_name: String,
    pub hidden: bool,
    pub erased: bool,
    pub source: Source,
}

impl Summary {
    /// The summary of `entrants` (in the order they entered, as
    /// [`entrants::list`] reads them), with the names of
    /// those in at or before `names_before`, and `drawn` the IDs of the
    /// entrants in a draw.
    pub fn of<'a>(
        entrants: &[EntrantRow],
        names_before: i64,
        drawn: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let drawn: std::collections::HashSet<&str> = drawn.into_iter().collect();
        let mut drawn: Vec<DrawnEntrant> = entrants
            .iter()
            .filter(|e| drawn.contains(e.id.as_str()))
            .map(|e| DrawnEntrant {
                id: e.id.clone(),
                display_name: e.display_name.clone(),
                hidden: e.hidden,
                erased: e.erased,
                source: e.source,
            })
            .collect();
        drawn.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            count: entrants.len() as u64,
            unerased: entrants.iter().filter(|e| !e.erased).count() as u64,
            recent_names: entrants
                .iter()
                .rev()
                .filter(|e| !e.hidden && !e.erased && e.created_at <= names_before)
                .take(RECENT_NAMES)
                .map(|e| e.display_name.clone())
                .collect(),
            drawn,
            list_added: entrants
                .iter()
                .any(|e| e.source == Source::Manual && e.batch_row.is_some()),
        }
    }
}

/// A raffle, its prizes, the [`Summary`] of its entrants and the raffle as
/// drawn, read in one [`Db::batch_rows`] like [`Full`], without reading
/// every entrant: what the projector and the admin poll, every few seconds
/// for the whole evening.
#[derive(Debug, Clone)]
pub struct Lite {
    pub raffle: RaffleRow,
    pub prizes: Vec<PrizeRow>,
    pub entrants: Summary,
    pub drawn: Option<Raffle>,
}

#[derive(Deserialize)]
struct Counts {
    count: i64,
    unerased: i64,
    list_added: i64,
}

impl Lite {
    /// Reads the raffle, or `None` if it doesn't exist, with the names of
    /// entrants in at or before `names_before`.
    pub async fn read(db: &Db, raffle_id: &str, names_before: i64) -> Result<Option<Self>> {
        let mut sets = db
            .batch_rows(vec![
                db.query("SELECT * FROM raffles WHERE id = ?1").bind(raffle_id),
                db.query(format!(
                    "SELECT * FROM prizes WHERE raffle_id = ?1 {}",
                    prizes::ORDER
                ))
                .bind(raffle_id),
                db.query(format!(
                    "SELECT COUNT(*) AS count,
                            COUNT(CASE WHEN erased = 0 THEN 1 END) AS unerased,
                            COUNT(CASE WHEN source = '{manual}' AND batch_row IS NOT NULL THEN 1 END) AS list_added
                     FROM entrants WHERE raffle_id = ?1",
                    manual = Source::Manual.as_str(),
                ))
                .bind(raffle_id),
                // Newest first: `entrants::ORDER` backwards.
                db.query(format!(
                    "SELECT display_name FROM entrants
                     WHERE raffle_id = ?1 AND hidden = 0 AND erased = 0 AND created_at <= ?2
                     ORDER BY created_at DESC, COALESCE(batch_row, 0) DESC, id DESC
                     LIMIT {RECENT_NAMES}"
                ))
                .bind(raffle_id)
                .bind(names_before),
                // From the draws, each entrant by its key: not a scan of
                // the raffle's entrants.
                db.query(
                    "SELECT DISTINCT e.id, e.display_name, e.hidden, e.erased, e.source
                     FROM draws d JOIN entrants e ON e.id = d.entrant_id
                     WHERE d.raffle_id = ?1 AND e.raffle_id = ?1
                     ORDER BY e.id",
                )
                .bind(raffle_id),
                raffles::events_query(db, raffle_id),
            ])
            .await?
            .into_iter();
        let (Some(raffle), Some(prizes), Some(counts), Some(names), Some(drawn), Some(events)) = (
            sets.next(),
            sets.next(),
            sets.next(),
            sets.next(),
            sets.next(),
            sets.next(),
        ) else {
            return Ok(None);
        };
        let Some(raffle) = raffle.into_iter().next() else {
            return Ok(None);
        };
        let raffle: RaffleRow = raffle.decode()?;
        let counts = match counts.into_iter().next() {
            Some(row) => row.decode::<Counts>()?,
            None => Counts {
                count: 0,
                unerased: 0,
                list_added: 0,
            },
        };
        let drawn_raffle = raffles::rebuild(raffle.opened_at, raffle.manifest.as_deref(), events)?;
        Ok(Some(Self {
            raffle,
            prizes: decode_all(prizes)?,
            entrants: Summary {
                count: counts.count.max(0) as u64,
                unerased: counts.unerased.max(0) as u64,
                recent_names: decode_all::<(String,)>(names)?
                    .into_iter()
                    .map(|(n,)| n)
                    .collect(),
                drawn: decode_all(drawn)?,
                list_added: counts.list_added > 0,
            },
            drawn: drawn_raffle,
        }))
    }
}
