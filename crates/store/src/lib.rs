//! Rafi's SQLite database, shared by the app and the `restate-rafi` draw engine.
//!
//! The app owns the schema ([`migrate`]) and everything in it; the draw engine
//! only ever writes a raffle's draw state, through [`save_draw_state`]. Both
//! processes open the same file in WAL mode, which SQLite supports for several
//! processes on one host.

use std::{str::FromStr, time::Duration};

use rafi_core::state::{DrawState, Event};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous},
};

pub use sqlx::Error;

/// Connection options for a database URL (`sqlite://rafi.db`, `sqlite::memory:`).
///
/// File databases use WAL, so the app and the draw engine can write the same
/// file, and wait up to five seconds for each other's locks.
pub fn options(url: &str) -> Result<SqliteConnectOptions, Error> {
    let opts = SqliteConnectOptions::from_str(url)?
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    Ok(if url.contains(":memory:") {
        opts
    } else {
        opts.journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
    })
}

/// Brings the schema up to date. Only the app runs this.
pub async fn migrate(db: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(db).await
}

/// Stores a draw engine's state for a raffle.
///
/// Idempotent and safe to retry: a state is only written when it is newer than
/// the stored one, and only while `engine` still owns the raffle's draw (a
/// raffle switched to the local engine ignores late writes from Restate).
/// Moves the raffle to `drawing` or `finished` as the state progresses.
///
/// Returns whether the state was written.
pub async fn save_draw_state(
    db: &SqlitePool,
    state: &DrawState,
    engine: &str,
) -> Result<bool, Error> {
    let json = serde_json::to_string(state).map_err(|e| Error::Encode(Box::new(e)))?;
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let written = sqlx::query(
        "UPDATE raffles SET
            draw_state = ?1,
            draw_version = ?2,
            state = CASE WHEN state IN ('frozen', 'drawing') THEN ?3 ELSE state END,
            finished_at = CASE WHEN ?4 AND finished_at IS NULL THEN ?5 ELSE finished_at END,
            updated_at = ?5
         WHERE id = ?6 AND engine = ?7 AND list_hash = ?8 AND draw_version < ?2",
    )
    .bind(&json)
    .bind(state.version as i64)
    .bind(lifecycle)
    .bind(state.finished)
    .bind(now)
    .bind(&state.raffle_id)
    .bind(engine)
    .bind(&state.list_hash)
    .execute(db)
    .await?
    .rows_affected();
    Ok(written > 0)
}

#[cfg(test)]
mod tests {
    use rafi_core::commit::{CommitDoc, UnitRef};
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn db() -> SqlitePool {
        let db = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options("sqlite::memory:").unwrap())
            .await
            .unwrap();
        migrate(&db).await.unwrap();
        db
    }

    async fn frozen(db: &SqlitePool, engine: &str) -> DrawState {
        let doc = CommitDoc::new(
            "r1",
            &rafi_core::drand::chains::QUICKNET,
            vec!["a".repeat(32), "b".repeat(32)],
            vec![UnitRef {
                unit_id: "u1".into(),
                item_id: "i1".into(),
                item_name: "Ticket".into(),
            }],
        );
        let state = DrawState::freeze(&doc);
        sqlx::query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, engine, list_hash, created_at, updated_at)
             VALUES ('r1', 'o', 's', 'j', 't', 'frozen', ?, ?, 0, 0)",
        )
        .bind(engine)
        .bind(&state.list_hash)
        .execute(db)
        .await
        .unwrap();
        state
    }

    #[tokio::test]
    async fn keeps_the_newest_state_of_the_owning_engine() {
        let db = db().await;
        let mut state = frozen(&db, "restate").await;

        assert!(
            !save_draw_state(&db, &state, "local").await.unwrap(),
            "another engine's raffle"
        );
        assert!(save_draw_state(&db, &state, "restate").await.unwrap());
        assert!(
            !save_draw_state(&db, &state, "restate").await.unwrap(),
            "a retry is a no-op"
        );

        state.finish().unwrap();
        assert!(save_draw_state(&db, &state, "restate").await.unwrap());
        let (lifecycle, finished_at): (String, Option<i64>) =
            sqlx::query_as("SELECT state, finished_at FROM raffles WHERE id = 'r1'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(lifecycle, "finished");
        assert!(finished_at.is_some());

        let mut other = state.clone();
        other.list_hash = "0".repeat(64);
        other.version += 1;
        assert!(
            !save_draw_state(&db, &other, "restate").await.unwrap(),
            "a different entrant list"
        );
    }
}
