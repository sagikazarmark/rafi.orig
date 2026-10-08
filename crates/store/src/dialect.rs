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

/// A subquery listing the `entrant_id`s of the objects in a draw state's
/// `list` (`events` or `units`) whose `field` equals `value`. `draw_state` is
/// an SQL expression for the draw state's JSON text: a column (`r.draw_state`)
/// or a scalar subquery.
///
/// `list`, `field` and `value` are SQL literals written by the store itself,
/// never input.
pub(crate) fn draw_state_entrants(
    dialect: Dialect,
    draw_state: &str,
    list: &str,
    field: &str,
    value: &str,
) -> String {
    match dialect {
        Dialect::Sqlite => format!(
            "SELECT json_extract(x.value, '$.entrant_id') FROM json_each({draw_state}, '$.{list}') x
             WHERE json_extract(x.value, '$.{field}') = '{value}'"
        ),
        // `draw_state` is TEXT, so it is cast to `jsonb` (NULL stays NULL, and
        // a NULL list has no elements).
        Dialect::Postgres => format!(
            "SELECT x.value ->> 'entrant_id' FROM jsonb_array_elements(({draw_state})::jsonb -> '{list}') AS x(value)
             WHERE x.value ->> '{field}' = '{value}'"
        ),
    }
}
