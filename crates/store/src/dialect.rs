//! The database dialects. The store's SQL is plain SQL that SQLite (and D1)
//! and Postgres both accept; a statement that can't be shared would choose
//! its text by [`Db::dialect`](crate::Db::dialect), here.

/// The SQL dialect of a [`Db`](crate::Db)'s backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// SQLite, natively, and Cloudflare D1.
    Sqlite,
    /// PostgreSQL (native, self-hosted).
    Postgres,
}
