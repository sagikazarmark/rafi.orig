//! The query layer: [`Db`] and [`Query`], over whichever backend the target uses.
//!
//! The shape follows sqlx (`db.query(sql).bind(x).all().await`), so the SQL and
//! its call sites read the same on every backend. Statements use `?` or `?N`
//! placeholders; SQLite and D1 accept them, and the Postgres backend
//! rewrites them to `$N`.

use std::sync::Arc;

use rafi_engine::BoxFuture;
use serde::de::DeserializeOwned;

use crate::{
    backend::Backend,
    dialect::Dialect,
    error::{Error, Result},
    value::{Row, Value},
};

/// A handle to the database. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    pub(crate) backend: Backend,
    /// Awaited before each statement or batch is sent, with its SQL (a
    /// batch's first statement): a test's pause point (`Db::with_hook`, the
    /// `hooks` feature).
    /// `None` outside tests.
    pub(crate) hook: Option<Hook>,
}

/// What `Db::with_hook` (the `hooks` feature) runs before each statement: given its SQL, a
/// future the statement waits for.
pub type Hook = Arc<dyn Fn(&str) -> BoxFuture<'static, ()> + Send + Sync>;

impl Db {
    pub(crate) fn from_backend(backend: Backend) -> Self {
        Self {
            backend,
            hook: None,
        }
    }

    /// This database through a handle that awaits `hook` before each
    /// statement it sends (a batch: before the batch, with its first
    /// statement's SQL). For tests that run a second actor between two of a
    /// use case's statements: the hook pauses on the statement it is waiting
    /// for while the other actor, on a handle without it, goes ahead. Other
    /// clones of `self` are unaffected.
    #[cfg(feature = "hooks")]
    pub fn with_hook(&self, hook: Hook) -> Self {
        Self {
            backend: self.backend.clone(),
            hook: Some(hook),
        }
    }

    /// Before `stmts` (one statement, or a batch) are sent: the hook, with
    /// the first one's SQL; in this crate's tests, the capture
    /// ([`capture`]) sees each of them.
    async fn before(&self, stmts: &[Statement]) {
        #[cfg(all(test, not(target_arch = "wasm32")))]
        capture::record(stmts);
        if let (Some(hook), Some(first)) = (&self.hook, stmts.first()) {
            hook(&first.sql).await;
        }
    }

    /// The backend's SQL dialect, for the few statements that differ.
    pub fn dialect(&self) -> Dialect {
        self.backend.dialect()
    }

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
        let stmts: Vec<Statement> = queries.into_iter().map(|q| q.stmt).collect();
        self.before(&stmts).await;
        self.backend.batch(stmts).await
    }

    /// Runs reads as one transaction and returns each statement's rows: one
    /// consistent snapshot, which separate queries are not (a write can land
    /// between them). A write may be among them, returning no rows: the
    /// reads before it see exactly what it will change (a count that D1's
    /// change counts can't give, since they include triggers' writes).
    pub async fn batch_rows(&self, queries: Vec<Query<'_>>) -> Result<Vec<Vec<Row>>> {
        let stmts: Vec<Statement> = queries.into_iter().map(|q| q.stmt).collect();
        self.before(&stmts).await;
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
        self.db.before(std::slice::from_ref(&self.stmt)).await;
        self.db.backend.run(self.stmt).await
    }

    /// The raw rows.
    pub async fn rows(self) -> Result<Vec<Row>> {
        self.db.before(std::slice::from_ref(&self.stmt)).await;
        self.db.backend.all(self.stmt).await
    }
}

/// What a test of this crate's ran: every statement sent while
/// [`capture::statements`] awaits a future on this thread (a
/// `#[tokio::test]` runs on one), each batch's whole, for
/// `query_plans`.
#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) mod capture {
    use std::cell::RefCell;

    use super::Statement;

    thread_local! {
        static SENT: RefCell<Option<Vec<Statement>>> = const { RefCell::new(None) };
    }

    pub(super) fn record(stmts: &[Statement]) {
        SENT.with_borrow_mut(|sent| {
            if let Some(sent) = sent {
                sent.extend_from_slice(stmts);
            }
        });
    }

    /// Awaits `run`, and returns what it gave and the statements it sent.
    pub(crate) async fn statements<T>(run: impl Future<Output = T>) -> (T, Vec<Statement>) {
        SENT.with_borrow_mut(|sent| *sent = Some(Vec::new()));
        let out = run.await;
        let sent = SENT.with_borrow_mut(Option::take).unwrap_or_default();
        (out, sent)
    }
}
