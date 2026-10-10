//! Rafi's database, shared by the app, the drawing engine and the live channel.
//!
//! The schema lives in `migrations/` at the workspace root. Natively the
//! database is SQLite or, for self-hosting, PostgreSQL (through sqlx, chosen
//! by the URL's scheme), and [`Db::connect`] applies the migrations
//! (`migrations/` and `migrations/postgres/`); on Cloudflare Workers it is
//! D1, and `wrangler d1 migrations apply` applies the SQLite files.
//!
//! Postgres is the `postgres` feature, on by default; it adds nothing on
//! Workers.
//!
//! All of rafi's SQL lives here, as domain functions over a [`Db`]:
//!
//! - [`raffles`], [`prizes`], [`entrants`]: the organizer's and participants'
//!   reads and writes, the freeze's steps included;
//! - [`audit`], [`altcha`] (redeemed proof-of-work challenges), [`retention`]
//!   (erasure, ending live channels, `meta`);
//! - [`view::Full`]: what the projector and the admin see, and
//!   [`live::Snapshot`]: what phones see, each read in one snapshot;
//! - [`save_drawing`], the drawing engine's only write; [`DrawWriter`] offers
//!   it as the engine's [`DrawStore`]. It carries no app policy, so the
//!   engine's hosts need none of the app's settings.
//!
//! The SQL is written for SQLite (and D1) and Postgres alike: `?`/`?N`
//! placeholders, no `rowid`, booleans stored as 0/1 and compared explicitly.
//! What can't be shared lives in one place, `dialect`, chosen at runtime by
//! [`Db::dialect`].
//!
//! D1 has no interactive transactions: a write that spans several statements
//! goes through [`Db::batch`], which runs them as one transaction on every
//! backend. Every future is `Send`, on Workers too.
//!
//! What every backend must do is pinned down by the storage conformance
//! suite, `conformance` (the `conformance` feature): named scenarios that
//! `tests/conformance.rs` runs against SQLite and Postgres, and
//! `crates/store-d1-check` against D1.

pub mod altcha;
pub mod audit;
mod backend;
mod dialect;
pub mod entrants;
mod error;
pub mod live;
pub mod prizes;
mod query;
pub mod raffles;
pub mod retention;
mod value;
pub mod view;

#[cfg(not(target_arch = "wasm32"))]
pub use backend::options;
pub use dialect::Dialect;
pub use entrants::EntrantRow;
pub use error::{Error, Result};
pub use prizes::PrizeRow;
pub use query::{Db, Query};
pub use raffles::RaffleRow;
use rafi_drawing::Drawing;
use rafi_engine::BoxFuture;
use rafi_engine::DrawStore;
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

/// When personal data of a finished raffle is erased, counted from when it
/// finished: the app's retention settings ([`retention::date_purges`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgeAfter {
    pub nonwinners_secs: i64,
    pub winners_secs: i64,
}

impl PurgeAfter {
    pub fn days(nonwinners: i64, winners: i64) -> Self {
        Self {
            nonwinners_secs: nonwinners * 86_400,
            winners_secs: winners * 86_400,
        }
    }
}

/// Stores a drawing engine's state for a raffle.
///
/// Idempotent and safe to retry: a state is only written when it is newer than
/// the stored one, and for the entrant list the raffle was frozen with. The
/// raffle follows its drawing ([`rafi_lifecycle::drawing::lifecycle`]) to
/// `drawing` and `finished`, and its finish is timed. Nothing here is policy:
/// when personal data is erased is the app's to date
/// ([`retention::date_purges`]).
///
/// Returns whether the state was written.
pub async fn save_drawing(db: &Db, state: &Drawing) -> Result<bool> {
    let stored = StoredDrawing::of(state)?;
    let lifecycle = rafi_lifecycle::drawing::lifecycle(state).as_str();
    let written = db
        .query(format!(
            "UPDATE raffles SET
                drawing = ?1,
                drawn_entrants = ?7,
                winners = ?8,
                drawing_version = ?2,
                state = CASE WHEN state IN ({following}) THEN ?3 ELSE state END,
                finished_at = CASE WHEN ?4 = 1 AND finished_at IS NULL THEN ?5 ELSE finished_at END,
                updated_at = ?5
             WHERE id = ?6 AND manifest_hash = ?9 AND drawing_version < ?2",
            following = raffles::states(rafi_lifecycle::RaffleState::follows_drawing),
        ))
        .bind(stored.json)
        .bind(state.version())
        .bind(lifecycle)
        .bind(state.is_finished())
        .bind(now())
        .bind(state.id().as_str())
        .bind(stored.drawn_entrants)
        .bind(stored.winners)
        .bind(state.manifest_hash().to_string())
        .run()
        .await?;
    Ok(written > 0)
}

/// A drawing as the `raffles` row stores it: the drawing (its manifest and
/// event log), and the entrants the database itself needs from it, as JSON
/// arrays of IDs.
pub(crate) struct StoredDrawing {
    pub json: String,
    /// Every entrant drawn: only their names may leave the database.
    pub drawn_entrants: String,
    /// The entrants holding a claimed item: their data is kept longer.
    pub winners: String,
}

impl StoredDrawing {
    pub fn of(state: &Drawing) -> Result<Self> {
        Ok(Self {
            json: encode(state)?,
            drawn_entrants: encode(&state.drawn_entrants())?,
            winners: encode(&state.winners())?,
        })
    }
}

fn encode(value: &impl serde::Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|e| Error::Backend(e.to_string()))
}

/// Reads a stored drawing. One that doesn't load (another shape, or a log
/// that isn't its own) is an error, never "no drawing yet".
pub(crate) fn parse_drawing(json: &str) -> Result<Drawing> {
    serde_json::from_str(json).map_err(|e| Error::Decode(format!("the stored drawing: {e}")))
}

/// The drawing engine's [`DrawStore`] over this database: [`save_drawing`].
#[derive(Clone)]
pub struct DrawWriter {
    db: Db,
}

impl DrawWriter {
    pub fn new(db: Db) -> Self {
        Self { db }
    }
}

impl DrawStore for DrawWriter {
    fn save_drawing<'a>(&'a self, state: &'a Drawing) -> BoxFuture<'a, anyhow::Result<bool>> {
        Box::pin(async move {
            save_drawing(&self.db, state)
                .await
                .map_err(|e| anyhow::anyhow!("writing the draw state: {e}"))
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
    use super::*;

    #[tokio::test]
    async fn opening_lazily_does_not_create_the_file() {
        let path = std::env::temp_dir().join(format!("rafi-lazy-{}.db", std::process::id()));
        let db = Db::connect_lazy(&format!("sqlite://{}", path.display())).unwrap();
        assert!(!path.exists(), "the app creates the database");
        let writer = DrawWriter::new(db);
        let manifest = rafi_drawing::Manifest::new(
            "r1",
            rafi_drawing::BeaconSource::QUICKNET,
            vec!["a".repeat(32).into()],
            vec![rafi_drawing::ManifestItem {
                id: "u1".into(),
                name: "Book".into(),
            }],
        );
        let drawing = Drawing::open(&manifest).unwrap();
        assert!(
            writer.save_drawing(&drawing).await.is_err(),
            "no database yet: the engine retries"
        );
        assert!(!path.exists());
    }
}
