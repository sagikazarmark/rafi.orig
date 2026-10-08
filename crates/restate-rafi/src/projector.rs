//! Where the draw state goes after every change.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

use anyhow::{Context, bail};
use rafi_core::state::DrawState;
use rafi_store::{Db, PurgeAfter};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePoolOptions;

/// A boxed, sendable future (the projector is a trait object).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Receives the draw state after every change.
///
/// Called from a journaled `run`, so at least once: implementations must be
/// idempotent. [`DrawState::version`] increases with every change, so a
/// receiver can simply keep the highest version it has seen.
pub trait Projector: Send + Sync + 'static {
    fn project<'a>(&'a self, state: &'a DrawState) -> BoxFuture<'a, anyhow::Result<()>>;
}

/// The Rafi app's database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseConfig {
    /// The same SQLite database the app uses (its `RAFI_DATABASE_URL`).
    pub url: String,
    /// Days after a raffle finishes until non-winners are erased. Set it to the
    /// app's `RAFI_RETENTION_DAYS`: whoever stores the finished state dates it.
    pub retention_nonwinners_days: i64,
    /// Days after a raffle finishes until winners are erased (the app's
    /// `RAFI_RETENTION_WINNERS_DAYS`).
    pub retention_winners_days: i64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "sqlite://rafi.db".into(),
            retention_nonwinners_days: 30,
            retention_winners_days: 90,
        }
    }
}

impl DatabaseConfig {
    /// When personal data of a finished raffle is erased.
    pub fn purge_after(&self) -> PurgeAfter {
        PurgeAfter::days(self.retention_nonwinners_days, self.retention_winners_days)
    }
}

/// Writes the draw state into the Rafi app's SQLite database, where the app
/// picks it up and pushes it to the screens.
///
/// The app owns the schema and creates the file; until it has, writes fail and
/// Restate retries them.
#[derive(Clone)]
pub struct SqliteProjector {
    db: Db,
    purge: PurgeAfter,
}

impl SqliteProjector {
    /// Opens the database lazily: the first projection connects.
    pub fn new(cfg: &DatabaseConfig) -> anyhow::Result<Self> {
        if cfg.url.trim().is_empty() {
            bail!("database.url is required");
        }
        let opts = rafi_store::options(&cfg.url)
            .with_context(|| format!("invalid database.url {:?}", cfg.url))?
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_lazy_with(opts);
        Ok(Self::from_db(Db::from_pool(pool), cfg.purge_after()))
    }

    pub fn from_db(db: Db, purge: PurgeAfter) -> Self {
        Self { db, purge }
    }
}

impl Projector for SqliteProjector {
    fn project<'a>(&'a self, state: &'a DrawState) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let written = rafi_store::save_draw_state(&self.db, state, self.purge)
                .await
                .context("writing the draw state")?;
            if !written {
                // Already stored (a retry), or for another entrant list.
                tracing::debug!(
                    raffle_id = state.raffle_id,
                    version = state.version,
                    "draw state not written"
                );
            }
            Ok(())
        })
    }
}

/// Keeps every projected state in memory (tests, local experiments).
#[derive(Clone, Default)]
pub struct RecordingProjector {
    states: Arc<Mutex<Vec<DrawState>>>,
}

impl RecordingProjector {
    /// Every projection received so far, in order.
    pub fn states(&self) -> Vec<DrawState> {
        self.states.lock().expect("projector lock").clone()
    }

    /// The latest projection for `raffle_id`.
    pub fn latest(&self, raffle_id: &str) -> Option<DrawState> {
        self.states()
            .into_iter()
            .filter(|s| s.raffle_id == raffle_id)
            .max_by_key(|s| s.version)
    }
}

impl Projector for RecordingProjector {
    fn project<'a>(&'a self, state: &'a DrawState) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.states
                .lock()
                .expect("projector lock")
                .push(state.clone());
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_url_is_required() {
        assert!(
            SqliteProjector::new(&DatabaseConfig {
                url: " ".into(),
                ..DatabaseConfig::default()
            })
            .is_err()
        );
    }

    #[tokio::test]
    async fn opening_does_not_create_the_file() {
        let path = std::env::temp_dir().join(format!("rafi-projector-{}.db", std::process::id()));
        SqliteProjector::new(&DatabaseConfig {
            url: format!("sqlite://{}", path.display()),
            ..DatabaseConfig::default()
        })
        .unwrap();
        assert!(
            !path.exists(),
            "the app creates the database, not the draw engine"
        );
    }
}
