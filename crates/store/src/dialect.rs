//! The SQL that differs between database dialects. Everything else in the
//! store is plain SQL that SQLite (and D1) and Postgres both accept.

/// The SQL dialect of a [`Db`](crate::Db)'s backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// SQLite, natively, and Cloudflare D1.
    Sqlite,
    /// PostgreSQL (native, self-hosted).
    Postgres,
}

/// A subquery listing the entrant IDs in a JSON array column
/// (`raffles.drawn_entrants` or `raffles.winners`). `list` is an SQL
/// expression for the array's text: a column (`r.winners`) or a scalar
/// subquery, written by the store itself, never input.
pub(crate) fn entrants_in(dialect: Dialect, list: &str) -> String {
    match dialect {
        Dialect::Sqlite => format!("SELECT value FROM json_each({list})"),
        // The column is TEXT, so it is cast to `jsonb` (NULL stays NULL, and
        // has no elements).
        Dialect::Postgres => format!("SELECT jsonb_array_elements_text(({list})::jsonb)"),
    }
}
