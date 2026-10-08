//! The SQL that differs between database dialects. Everything else in the
//! store is plain SQL that SQLite (and D1) and Postgres both accept.

/// A subquery listing the `entrant_id`s of the objects in a draw state's
/// `list` (`events` or `units`) whose `field` equals `value`. `draw_state` is
/// an SQL expression for the draw state's JSON text: a column (`r.draw_state`)
/// or a scalar subquery.
///
/// `list`, `field` and `value` are SQL literals written by the store itself,
/// never input.
// SQLite dialect: `json_each` and `json_extract`. Postgres would read
// `jsonb_array_elements((draw_state)::jsonb -> 'list') x` with `x.value ->> 'field'`.
pub(crate) fn draw_state_entrants(
    draw_state: &str,
    list: &str,
    field: &str,
    value: &str,
) -> String {
    format!(
        "SELECT json_extract(x.value, '$.entrant_id') FROM json_each({draw_state}, '$.{list}') x
         WHERE json_extract(x.value, '$.{field}') = '{value}'"
    )
}
