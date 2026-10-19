//! SQLite through sqlx: the native default, for self-hosting, development and
//! tests.

use std::{str::FromStr, time::Duration};

use sqlx::{
    Column, Row as _, SqlitePool, TypeInfo, ValueRef,
    sqlite::{
        SqliteArguments, SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow,
        SqliteSynchronous,
    },
};

use crate::{
    error::{Error, Result},
    query::Statement,
    value::{Row, Value},
};

#[derive(Clone)]
pub(crate) struct Sqlite {
    pub(super) pool: SqlitePool,
}

/// Opens (creating if missing) the database at `url` (`sqlite://rafi.db`,
/// `sqlite::memory:`).
///
/// File databases use WAL, so the app and a self-hosted raffle engine can write
/// the same file, and wait up to five seconds for each other's locks. An
/// in-memory database exists per connection, so it keeps exactly one.
pub(super) async fn connect(url: &str) -> Result<Sqlite> {
    let in_memory = url.contains(":memory:");
    let opts = options(url)?.create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(if in_memory { 1 } else { 8 })
        .min_connections(if in_memory { 1 } else { 0 })
        .idle_timeout((!in_memory).then(|| Duration::from_secs(600)))
        .max_lifetime((!in_memory).then(|| Duration::from_secs(3600)))
        .connect_with(opts)
        .await?;
    Ok(Sqlite { pool })
}

/// Opens the existing database at `url` lazily, without creating the file.
pub(super) fn connect_lazy(url: &str) -> Result<Sqlite> {
    let opts = options(url)?.create_if_missing(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_lazy_with(opts);
    Ok(Sqlite { pool })
}

/// Connection options for an SQLite database URL.
pub(crate) fn options(url: &str) -> Result<SqliteConnectOptions> {
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

/// Statements come from rafi's own code with every value bound as a parameter,
/// never interpolated, which is what sqlx's audit marker asks for.
fn prepare(stmt: &Statement) -> sqlx::query::Query<'_, sqlx::Sqlite, SqliteArguments> {
    let mut q = sqlx::query(sqlx::AssertSqlSafe(stmt.sql.as_str()));
    for p in &stmt.params {
        q = match p {
            Value::Null => q.bind(None::<i64>),
            Value::Int(i) => q.bind(*i),
            Value::Real(f) => q.bind(*f),
            Value::Text(s) => q.bind(s.as_str()),
        };
    }
    q
}

fn to_row(row: &SqliteRow) -> Result<Row> {
    let mut out = Row::default();
    for (i, col) in row.columns().iter().enumerate() {
        out.columns.push(col.name().to_owned());
        let raw = row.try_get_raw(i)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" | "BOOLEAN" | "INT8" | "INT" | "BIGINT" => Value::Int(row.try_get(i)?),
                "REAL" | "FLOAT" | "DOUBLE" | "NUMERIC" => Value::Real(row.try_get(i)?),
                "BLOB" => {
                    let bytes: Vec<u8> = row.try_get(i)?;
                    Value::Text(String::from_utf8_lossy(&bytes).into_owned())
                }
                _ => Value::Text(row.try_get(i)?),
            }
        };
        out.values.push(value);
    }
    Ok(out)
}

impl Sqlite {
    pub(crate) async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../migrations/sqlite")
            .run(&self.pool)
            .await
            .map_err(|e| Error::Backend(e.to_string()))
    }

    pub(crate) async fn all(&self, stmt: Statement) -> Result<Vec<Row>> {
        prepare(&stmt)
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(to_row)
            .collect()
    }

    pub(crate) async fn run(&self, stmt: Statement) -> Result<u64> {
        Ok(prepare(&stmt).execute(&self.pool).await?.rows_affected())
    }

    pub(crate) async fn batch(&self, stmts: Vec<Statement>) -> Result<Vec<u64>> {
        let mut tx = self.pool.begin().await?;
        let mut changes = Vec::with_capacity(stmts.len());
        for stmt in &stmts {
            changes.push(prepare(stmt).execute(&mut *tx).await?.rows_affected());
        }
        tx.commit().await?;
        Ok(changes)
    }

    pub(crate) async fn batch_rows(&self, stmts: Vec<Statement>) -> Result<Vec<Vec<Row>>> {
        let mut tx = self.pool.begin().await?;
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in &stmts {
            let rows = prepare(stmt).fetch_all(&mut *tx).await?;
            out.push(rows.iter().map(to_row).collect::<Result<_>>()?);
        }
        tx.commit().await?;
        Ok(out)
    }
}
