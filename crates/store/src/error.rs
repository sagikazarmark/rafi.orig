//! The store's error type, the same for every backend.

/// A database error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A `UNIQUE` (or primary key) constraint rejected the write.
    #[error("unique constraint violated: {0}")]
    Unique(String),
    /// [`Query::one`](crate::Query::one) found no row.
    #[error("no row found")]
    NotFound,
    /// A row did not match the type it was decoded into.
    #[error("decoding a row: {0}")]
    Decode(String),
    /// Anything else the backend reported.
    #[error("database: {0}")]
    Backend(String),
}

impl Error {
    /// Whether this is a `UNIQUE` constraint violation.
    pub fn is_unique_violation(&self) -> bool {
        matches!(self, Self::Unique(_))
    }

    /// Classifies a backend's error message. SQLite and D1 both report
    /// constraint violations as `UNIQUE constraint failed: <table>.<column>`.
    pub(crate) fn from_message(msg: String) -> Self {
        if msg.contains("UNIQUE constraint failed") {
            Self::Unique(msg)
        } else {
            Self::Backend(msg)
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
