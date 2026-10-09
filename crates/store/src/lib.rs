//! Rafi's database, shared by the app, the draw engine and the live channel.
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
//! - [`raffles`], [`prizes`], [`entrants`]: the organizer's and attendees'
//!   reads and writes, the freeze's steps included;
//! - [`audit`], [`altcha`] (redeemed proof-of-work challenges), [`retention`]
//!   (erasure, ending live channels, `meta`);
//! - [`view::Full`]: what the projector and the admin see, and
//!   [`live::Snapshot`]: what phones see, each read in one snapshot;
//! - [`save_drawing`], the draw engine's only write; [`DrawWriter`] offers
//!   it as the engine's [`DrawStore`].
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
use rafi_drawing::drawing::{Drawing, Event};
use rafi_platform::{BoxFuture, DrawStore};
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

/// When personal data of a finished raffle is erased, counted from when it finished.
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

/// Stores a draw engine's state for a raffle.
///
/// Idempotent and safe to retry: a state is only written when it is newer than
/// the stored one, and for the entrant list the raffle was frozen with. Moves
/// the raffle to `drawing` or `finished` as the state progresses, and when it
/// finishes, dates the erasure of personal data.
///
/// Returns whether the state was written.
pub async fn save_drawing(db: &Db, state: &Drawing, purge: PurgeAfter) -> Result<bool> {
    let json = serde_json::to_string(state).map_err(|e| Error::Backend(e.to_string()))?;
    let started = state
        .events
        .iter()
        .any(|e| matches!(e, Event::Announce { .. } | Event::Draw { .. }));
    let lifecycle = if state.finished {
        "finished"
    } else if started {
        "drawing"
    } else {
        "frozen"
    };
    let written = db
        .query(
            "UPDATE raffles SET
                drawing = ?1,
                drawing_version = ?2,
                state = CASE WHEN state IN ('frozen', 'drawing') THEN ?3 ELSE state END,
                finished_at = CASE WHEN ?4 = 1 AND finished_at IS NULL THEN ?5 ELSE finished_at END,
                purge_nonwinners_at = CASE WHEN ?4 = 1 AND purge_nonwinners_at IS NULL
                    THEN COALESCE(finished_at, ?5) + ?6 ELSE purge_nonwinners_at END,
                purge_winners_at = CASE WHEN ?4 = 1 AND purge_winners_at IS NULL
                    THEN COALESCE(finished_at, ?5) + ?7 ELSE purge_winners_at END,
                updated_at = ?5
             WHERE id = ?8 AND manifest_hash = ?9 AND drawing_version < ?2",
        )
        .bind(json)
        .bind(state.version)
        .bind(lifecycle)
        .bind(state.finished)
        .bind(now())
        .bind(purge.nonwinners_secs)
        .bind(purge.winners_secs)
        .bind(&state.id)
        .bind(&state.manifest_hash)
        .run()
        .await?;
    Ok(written > 0)
}

/// The draw engine's [`DrawStore`] over this database: [`save_drawing`]
/// with the configured erasure dates.
#[derive(Clone)]
pub struct DrawWriter {
    db: Db,
    purge: PurgeAfter,
}

impl DrawWriter {
    pub fn new(db: Db, purge: PurgeAfter) -> Self {
        Self { db, purge }
    }
}

impl DrawStore for DrawWriter {
    fn save_drawing<'a>(&'a self, state: &'a Drawing) -> BoxFuture<'a, anyhow::Result<bool>> {
        Box::pin(async move {
            save_drawing(&self.db, state, self.purge)
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
        let writer = DrawWriter::new(db, PurgeAfter::days(30, 90));
        let doc = rafi_drawing::manifest::Manifest::new(
            "r1",
            rafi_drawing::BeaconSource::Drand(rafi_drawing::drand::chains::QUICKNET_HASH),
            vec!["a".repeat(32)],
            vec![],
        );
        assert!(
            writer.save_drawing(&Drawing::open(&doc)).await.is_err(),
            "no database yet: the engine retries"
        );
        assert!(!path.exists());
    }
}
