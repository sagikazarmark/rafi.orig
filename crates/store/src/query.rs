//! The query layer: [`Db`] and [`Query`], over whichever backend the target uses.
//!
//! The shape follows sqlx (`db.query(sql).bind(x).all().await`), so the SQL and
//! its call sites read the same on every backend. Statements use `?` or `?N`
//! placeholders; both SQLite and D1 accept them.

use serde::de::DeserializeOwned;

use crate::{
    backend::Backend,
    error::{Error, Result},
    value::{Row, Value},
};

/// A handle to the database. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    pub(crate) backend: Backend,
}

impl Db {
    /// Starts a statement.
    pub fn query(&self, sql: impl Into<String>) -> Query<'_> {
        Query {
            db: self,
            stmt: Statement {
                sql: sql.into(),
                params: Vec::new(),
            },
        }
    }

    /// Runs the statements in order as one transaction: all of them apply, or
    /// none. Returns each statement's number of changed rows (zero versus
    /// non-zero only, as with [`Query::run`]).
    ///
    /// D1 has no interactive transactions, so this is the only way to make
    /// several writes atomic. Use it for every write that spans statements.
    pub async fn batch(&self, queries: Vec<Query<'_>>) -> Result<Vec<u64>> {
        let stmts = queries.into_iter().map(|q| q.stmt).collect();
        self.backend.batch(stmts).await
    }

    /// Runs reads as one transaction and returns each statement's rows: one
    /// consistent snapshot, which separate queries are not (a write can land
    /// between them).
    pub async fn batch_rows(&self, queries: Vec<Query<'_>>) -> Result<Vec<Vec<Row>>> {
        let stmts = queries.into_iter().map(|q| q.stmt).collect();
        self.backend.batch_rows(stmts).await
    }
}

/// A statement and its parameters.
#[derive(Debug, Clone)]
pub(crate) struct Statement {
    pub sql: String,
    pub params: Vec<Value>,
}

/// A statement being built. Run it with one of the async methods.
#[must_use = "a query does nothing until it is run"]
pub struct Query<'a> {
    db: &'a Db,
    stmt: Statement,
}

impl Query<'_> {
    /// Binds the next positional parameter.
    pub fn bind(mut self, value: impl Into<Value>) -> Self {
        self.stmt.params.push(value.into());
        self
    }

    /// Every row, decoded.
    pub async fn all<T: DeserializeOwned>(self) -> Result<Vec<T>> {
        self.rows().await?.into_iter().map(Row::decode).collect()
    }

    /// The first row, if any.
    pub async fn optional<T: DeserializeOwned>(self) -> Result<Option<T>> {
        self.rows()
            .await?
            .into_iter()
            .next()
            .map(Row::decode)
            .transpose()
    }

    /// The first row; [`Error::NotFound`] if there is none.
    pub async fn one<T: DeserializeOwned>(self) -> Result<T> {
        self.optional().await?.ok_or(Error::NotFound)
    }

    /// Runs a write. Returns the number of changed rows.
    ///
    /// Only rely on zero versus non-zero: D1 also counts rows that triggers
    /// changed (every raffle write bumps `raffles.version`), SQLite does not.
    pub async fn run(self) -> Result<u64> {
        self.db.backend.run(self.stmt).await
    }

    /// The raw rows.
    pub async fn rows(self) -> Result<Vec<Row>> {
        self.db.backend.all(self.stmt).await
    }
}
