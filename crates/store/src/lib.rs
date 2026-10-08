//! Rafi's database, shared by the app, the draw engine and the live channel.
//!
//! The schema lives in `migrations/` at the workspace root. Natively the
//! database is SQLite (through sqlx), and [`Db::connect`] applies the
//! migrations; on Cloudflare Workers it is D1, and `wrangler d1 migrations
//! apply` applies the same files.
//!
//! The draw engine only ever writes a raffle's draw state, through
//! [`save_draw_state`].
//!
//! D1 has no interactive transactions: a write that spans several statements
//! goes through [`Db::batch`], which runs them as one transaction on both
//! backends.

mod backend;
mod error;
mod query;
mod value;

#[cfg(not(target_arch = "wasm32"))]
pub use backend::options;
pub use error::{Error, Result};
pub use query::{Db, Query};
use rafi_core::state::{DrawState, Event};
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
pub async fn save_draw_state(db: &Db, state: &DrawState, purge: PurgeAfter) -> Result<bool> {
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
                draw_state = ?1,
                draw_version = ?2,
                state = CASE WHEN state IN ('frozen', 'drawing') THEN ?3 ELSE state END,
                finished_at = CASE WHEN ?4 AND finished_at IS NULL THEN ?5 ELSE finished_at END,
                purge_nonwinners_at = CASE WHEN ?4 AND purge_nonwinners_at IS NULL
                    THEN COALESCE(finished_at, ?5) + ?6 ELSE purge_nonwinners_at END,
                purge_winners_at = CASE WHEN ?4 AND purge_winners_at IS NULL
                    THEN COALESCE(finished_at, ?5) + ?7 ELSE purge_winners_at END,
                updated_at = ?5
             WHERE id = ?8 AND list_hash = ?9 AND draw_version < ?2",
        )
        .bind(json)
        .bind(state.version)
        .bind(lifecycle)
        .bind(state.finished)
        .bind(now())
        .bind(purge.nonwinners_secs)
        .bind(purge.winners_secs)
        .bind(&state.raffle_id)
        .bind(&state.list_hash)
        .run()
        .await?;
    Ok(written > 0)
}

#[cfg(any(test, feature = "testsuite"))]
pub mod testsuite;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// The shared scenarios, against in-memory SQLite. `crates/store-d1-check`
    /// runs the same ones against D1.
    #[tokio::test]
    async fn testsuite() {
        for (name, result) in
            testsuite::run(|| async { Db::connect("sqlite::memory:").await.unwrap() }).await
        {
            if let Err(e) = result {
                panic!("{name}: {e}");
            }
        }
    }
}
