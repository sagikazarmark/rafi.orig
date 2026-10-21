//! Rafi's database, shared by the app, the raffle engine and the live channel.
//!
//! The schema lives in `migrations/` at the workspace root, one directory per
//! dialect. Natively the database is SQLite or, for self-hosting, PostgreSQL
//! (through sqlx, chosen by the URL's scheme), and [`Db::connect`] applies the
//! migrations (`migrations/sqlite/` and `migrations/postgres/`); on Cloudflare
//! Workers it is D1, and `wrangler d1 migrations apply` applies the SQLite
//! files.
//!
//! Postgres is the `postgres` feature, on by default; it adds nothing on
//! Workers.
//!
//! All of rafi's SQL lives here, as domain functions over a [`Db`]:
//!
//! - [`raffles`], [`prizes`], [`entrants`]: the organizer's and participants'
//!   reads and writes, the freeze's steps included; [`defaults`]: what an
//!   owner's new raffles start with; [`legal`]: which terms they accepted;
//! - [`audit`], [`altcha`] (redeemed proof-of-work challenges),
//!   [`housekeeping`] (ending live channels, when the run last succeeded) and
//!   [`retention`] (its erasure writes);
//! - [`view::Full`]: what the projector and the admin see, and
//!   [`live::snapshot`]: what phones see, each read in one snapshot;
//! - [`save_raffle`], the raffle engine's only write; [`RaffleWriter`] offers
//!   it as the engine's [`Store`](rafi_engine::Store). It carries no app policy, so the
//!   engine's hosts need none of the app's settings.
//!
//! A raffle is stored as its manifest (`raffles.manifest`, the string that is
//! hashed and published) and its events (`raffle_events`, one row per event,
//! append-only), and is rebuilt from them ([`raffles::drawn`]). Its completed
//! draws are rows too (`draws`), for what SQL needs from them: which entrants
//! were drawn, and which won.
//!
//! The SQL is written for SQLite (and D1) and Postgres alike: `?`/`?N`
//! placeholders, no `rowid`, booleans stored as 0/1 and compared explicitly.
//! None of it differs between them; a statement that had to would choose its
//! text by [`Db::dialect`].
//!
//! D1 has no interactive transactions: a write that spans several statements
//! goes through [`Db::batch`], which runs them as one transaction on every
//! backend. Every future is `Send`, on Workers too.
//!
//! What every backend must do is pinned down by the storage conformance
//! suite, `conformance` (the `conformance` feature): named scenarios that
//! `tests/conformance.rs` runs against SQLite and Postgres, and
//! `testbeds/store-d1` against D1.

pub mod altcha;
pub mod audit;
mod backend;
pub mod defaults;
mod dialect;
pub mod entrants;
mod error;
pub mod housekeeping;
pub mod legal;
pub mod live;
pub mod prizes;
mod query;
pub mod raffles;
pub mod retention;
mod value;
pub mod view;

pub use dialect::Dialect;
pub use entrants::EntrantRow;
pub use error::{Error, Result};
pub use prizes::PrizeRow;
pub use query::{Db, Query};
pub use raffles::RaffleRow;
use rafi_engine::BoxFuture;
use rafi_raffle::{Event, Raffle};
use std::collections::BTreeSet;
pub use value::{Row, Value};

/// The current time in unix seconds. `std::time` aborts on
/// `wasm32-unknown-unknown`, so Workers read the JavaScript clock.
pub fn now() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64)
    }
}

/// The current time in unix milliseconds; see [`now`].
pub fn now_ms() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }
}

/// Stores a raffle as its engine drew it, after every change: the events it
/// recorded since the last write (`raffle_events`), the draws they touched
/// (`draws`), and the row (`raffles`: opened, the lifecycle, the finish).
///
/// Idempotent and safe to retry: a state is only written when it is newer than
/// the stored one, and for the entrant list the raffle was frozen with. The
/// lifecycle follows the raffle ([`rafi_lifecycle::raffle::lifecycle`]) to
/// `drawing` and `finished`, and its finish is timed. Nothing here is policy:
/// erasing personal data is the app's.
///
/// It reads before it writes, to know which events are new; the write is one
/// [`Db::batch`]. A stale save that passes the read (a newer one landed in
/// between) carries the same events at the same positions, so its events
/// are already stored, and the batch's guard (the stored log is exactly this
/// state's) leaves the draws and the row alone.
///
/// Returns whether the state was written.
pub async fn save_raffle(db: &Db, state: &Raffle) -> Result<bool> {
    #[derive(serde::Deserialize)]
    struct Stored {
        manifest_hash: Option<String>,
        opened_at: Option<i64>,
        /// How many events are stored: the highest `seq`, or 0.
        events: i64,
    }

    let id = state.id().as_str();
    let manifest_hash = state.manifest_hash().to_string();
    let Some(stored) = db
        .query(
            "SELECT manifest_hash, opened_at,
                    (SELECT COALESCE(MAX(seq), 0) FROM raffle_events WHERE raffle_id = ?1) AS events
             FROM raffles WHERE id = ?1",
        )
        .bind(id)
        .optional::<Stored>()
        .await?
    else {
        return Ok(false);
    };
    let events = state.events();
    let stored_events = usize::try_from(stored.events).unwrap_or(0);
    if stored.manifest_hash.as_deref() != Some(manifest_hash.as_str())
        || (stored.opened_at.is_some() && stored_events >= events.len())
    {
        return Ok(false);
    }
    write_raffle(db, state, stored_events).await
}

/// [`save_raffle`]'s write, once its read found `stored_events` events
/// stored: one [`Db::batch`] that appends the rest of `state`'s events,
/// upserts the draws they touch and updates the row, all guarded on the
/// stored log being exactly `state`'s once the events are in. Returns whether
/// the row was updated.
pub(crate) async fn write_raffle(db: &Db, state: &Raffle, stored_events: usize) -> Result<bool> {
    let id = state.id().as_str();
    let events = state.events();
    let new = events.get(stored_events..).unwrap_or_default();
    let now = now();
    // The stored log is exactly this state's: evaluated after the inserts, so
    // a stale save touches no draw and leaves the row alone.
    let guard = "(SELECT COALESCE(MAX(seq), 0) FROM raffle_events WHERE raffle_id = ?1) = ?2";
    let mut writes = Vec::new();
    for (chunk, rows) in new.chunks(EVENTS_PER_INSERT).enumerate() {
        let mut insert = db.query(format!(
            "INSERT INTO raffle_events (raffle_id, seq, type, body, recorded_at) VALUES {}
             ON CONFLICT DO NOTHING",
            vec!["(?, ?, ?, ?, ?)"; rows.len()].join(", ")
        ));
        for (i, event) in rows.iter().enumerate() {
            let seq = stored_events + chunk * EVENTS_PER_INSERT + i + 1;
            insert = insert
                .bind(id)
                .bind(seq)
                .bind(event.kind())
                .bind(encode(event)?)
                .bind(now);
        }
        writes.push(insert);
    }
    let touched: BTreeSet<u64> = new
        .iter()
        .filter_map(|event| match event {
            Event::Drawn { k, .. } | Event::Resolved { k, .. } => Some(*k),
            _ => None,
        })
        .collect();
    for draw in state.draws().iter().filter(|d| touched.contains(&d.k)) {
        writes.push(
            db.query(format!(
                "INSERT INTO draws (raffle_id, k, item_id, entrant_id, outcome)
                 SELECT ?1, ?3, ?4, ?5, ?6 WHERE {guard}
                 ON CONFLICT (raffle_id, k) DO UPDATE SET outcome = excluded.outcome"
            ))
            .bind(id)
            .bind(events.len())
            .bind(draw.k)
            .bind(draw.item.as_str())
            .bind(draw.entrant.as_str())
            .bind(draw.outcome.as_str()),
        );
    }
    writes.push(
        db.query(format!(
            "UPDATE raffles SET
                state = CASE WHEN state IN ({following}) THEN ?4 ELSE state END,
                finished_at = CASE WHEN ?5 = 1 AND finished_at IS NULL THEN ?6 ELSE finished_at END,
                opened_at = COALESCE(opened_at, ?6),
                updated_at = ?6
             WHERE id = ?1 AND manifest_hash = ?3 AND {guard}",
            following = raffles::states(rafi_lifecycle::RaffleState::follows_raffle),
        ))
        .bind(id)
        .bind(events.len())
        .bind(state.manifest_hash().to_string())
        .bind(rafi_lifecycle::raffle::lifecycle(state).as_str())
        .bind(state.is_finished())
        .bind(now),
    );
    let changed = db.batch(writes).await?;
    Ok(changed.last().is_some_and(|&n| n > 0))
}

/// Events per `INSERT`: five parameters each, and D1 binds at most 100 per
/// statement.
const EVENTS_PER_INSERT: usize = 20;

fn encode(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::Backend(e.to_string()))
}

/// The raffle engine's [`Store`](rafi_engine::Store) over this database: [`save_raffle`].
#[derive(Clone)]
pub struct RaffleWriter {
    db: Db,
}

impl RaffleWriter {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

impl rafi_engine::Store for RaffleWriter {
    fn save_raffle<'a>(&'a self, state: &'a Raffle) -> BoxFuture<'a, anyhow::Result<bool>> {
        Box::pin(async move {
            save_raffle(&self.db, state)
                .await
                .map_err(|e| anyhow::anyhow!("writing the raffle: {e}"))
        })
    }
}

#[cfg(feature = "conformance")]
pub mod conformance;

/// Test support for suites that run against Postgres when
/// `RAFI_TEST_POSTGRES_URL` names a (throwaway) database.
#[cfg(all(feature = "postgres", not(target_arch = "wasm32")))]
pub mod testing {
    pub use crate::backend::fresh_schema;

    /// Names a throwaway Postgres database for tests.
    pub const POSTGRES_URL_VAR: &str = "RAFI_TEST_POSTGRES_URL";

    /// The database `RAFI_TEST_POSTGRES_URL` names, if it is set.
    pub fn postgres_base_url() -> Option<String> {
        std::env::var(POSTGRES_URL_VAR)
            .ok()
            .filter(|s| !s.trim().is_empty())
    }

    /// A URL for a fresh, empty schema in that database, or `None` if
    /// `RAFI_TEST_POSTGRES_URL` isn't set. Panics if the schema can't be
    /// created.
    pub async fn postgres_url() -> Option<String> {
        let base = postgres_base_url()?;
        Some(
            fresh_schema(&base)
                .await
                .unwrap_or_else(|e| panic!("creating a schema in {POSTGRES_URL_VAR}: {e}")),
        )
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use rafi_engine::Store as _;

    use super::*;

    #[tokio::test]
    async fn opening_lazily_does_not_create_the_file() {
        let path = std::env::temp_dir().join(format!("rafi-lazy-{}.db", std::process::id()));
        let db = Db::connect_lazy(&format!("sqlite://{}", path.display())).unwrap();
        assert!(!path.exists(), "the app creates the database");
        let writer = RaffleWriter::new(db);
        let manifest = rafi_raffle::Manifest::new(
            "r1",
            rafi_raffle::BeaconSource::QUICKNET,
            vec!["a".repeat(32).into()],
            vec![rafi_raffle::ManifestItem {
                id: "u1".into(),
                name: "Book".into(),
            }],
        );
        let raffle = Raffle::open(&manifest).unwrap();
        assert!(
            writer.save_raffle(&raffle).await.is_err(),
            "no database yet: the engine retries"
        );
        assert!(!path.exists());
    }
}
