//! The native backends, chosen by the database URL's scheme: SQLite (the
//! default) or, with the `postgres` feature, PostgreSQL.

use sqlx::SqlitePool;

#[cfg(feature = "postgres")]
use super::postgres::{self, Postgres};
use super::sqlite::{self, Sqlite};
use crate::{
    dialect::Dialect,
    error::{Error, Result},
    query::{Db, Statement},
    value::Row,
};

#[derive(Clone)]
pub(crate) enum Native {
    Sqlite(Sqlite),
    #[cfg(feature = "postgres")]
    Postgres(Postgres),
}

/// Whether `url` names a Postgres database (`postgres://`, `postgresql://`).
/// Anything else is SQLite, as before Postgres was supported.
fn is_postgres(url: &str) -> bool {
    let scheme = url.split_once(':').map_or("", |(s, _)| s);
    scheme.eq_ignore_ascii_case("postgres") || scheme.eq_ignore_ascii_case("postgresql")
}

#[cfg(not(feature = "postgres"))]
fn no_postgres() -> Error {
    Error::Backend("rafi-store was built without the `postgres` feature".into())
}

impl Db {
    /// Opens and migrates the database at `url`.
    ///
    /// - `sqlite://rafi.db`, `sqlite::memory:`: SQLite, creating the file if
    ///   missing. File databases use WAL, so the app and a self-hosted draw
    ///   engine can write the same file, and wait up to five seconds for each
    ///   other's locks. An in-memory database exists per connection, so it
    ///   keeps exactly one.
    /// - `postgres://user:password@host/db` (or `postgresql://`): PostgreSQL;
    ///   the database must exist. Every transaction is serializable, as SQLite's
    ///   are, and retried when Postgres aborts it as a serialization failure.
    pub async fn connect(url: &str) -> Result<Self> {
        let backend = if is_postgres(url) {
            #[cfg(feature = "postgres")]
            {
                Native::Postgres(postgres::connect(url).await?)
            }
            #[cfg(not(feature = "postgres"))]
            return Err(no_postgres());
        } else {
            Native::Sqlite(sqlite::connect(url).await?)
        };
        let db = Self { backend };
        db.migrate().await?;
        Ok(db)
    }

    /// Opens the existing database at `url` lazily: the first query connects.
    /// Neither creates the database nor migrates it; that's the app's job, so
    /// a writer that starts first (the raffle engine) fails until the app has
    /// run, and is retried.
    pub fn connect_lazy(url: &str) -> Result<Self> {
        let backend = if is_postgres(url) {
            #[cfg(feature = "postgres")]
            {
                Native::Postgres(postgres::connect_lazy(url)?)
            }
            #[cfg(not(feature = "postgres"))]
            return Err(no_postgres());
        } else {
            Native::Sqlite(sqlite::connect_lazy(url)?)
        };
        Ok(Self { backend })
    }

    /// Wraps an existing SQLite pool (it must already be migrated).
    pub fn from_pool(pool: SqlitePool) -> Self {
        Self {
            backend: Native::Sqlite(Sqlite { pool }),
        }
    }

    /// Brings the schema up to date: `migrations/sqlite/` for SQLite (on D1,
    /// `wrangler d1 migrations apply` runs the same files instead), and
    /// `migrations/postgres/` for Postgres.
    pub async fn migrate(&self) -> Result<()> {
        match &self.backend {
            Native::Sqlite(b) => b.migrate().await,
            #[cfg(feature = "postgres")]
            Native::Postgres(b) => b.migrate().await,
        }
    }
}

impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => Self::Unique(d.to_string()),
            sqlx::Error::RowNotFound => Self::NotFound,
            _ => Self::from_message(e.to_string()),
        }
    }
}

impl Native {
    pub(crate) fn dialect(&self) -> Dialect {
        match self {
            Self::Sqlite(_) => Dialect::Sqlite,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => Dialect::Postgres,
        }
    }

    pub(crate) async fn all(&self, stmt: Statement) -> Result<Vec<Row>> {
        match self {
            Self::Sqlite(b) => b.all(stmt).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.all(stmt).await,
        }
    }

    pub(crate) async fn run(&self, stmt: Statement) -> Result<u64> {
        match self {
            Self::Sqlite(b) => b.run(stmt).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.run(stmt).await,
        }
    }

    pub(crate) async fn batch(&self, stmts: Vec<Statement>) -> Result<Vec<u64>> {
        match self {
            Self::Sqlite(b) => b.batch(stmts).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.batch(stmts).await,
        }
    }

    pub(crate) async fn batch_rows(&self, stmts: Vec<Statement>) -> Result<Vec<Vec<Row>>> {
        match self {
            Self::Sqlite(b) => b.batch_rows(stmts).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(b) => b.batch_rows(stmts).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_postgres;

    #[test]
    fn the_scheme_picks_the_backend() {
        assert!(is_postgres("postgres://rafi@localhost/rafi"));
        assert!(is_postgres("postgresql://rafi@localhost/rafi"));
        assert!(is_postgres("POSTGRES://rafi@localhost/rafi"));
        assert!(!is_postgres("sqlite://rafi.db"));
        assert!(!is_postgres("sqlite::memory:"));
        assert!(!is_postgres("rafi.db"));
    }
}
