//! PostgreSQL through sqlx, for self-hosting (not on Workers).
//!
//! The store's SQL is written for SQLite; this backend adapts it:
//!
//! - **Placeholders:** `?N` becomes `$N`, and each bare `?` the next `$N`.
//!   Only `?` is rewritten (outside string literals and quoted names), since
//!   `$` appears in SQL literals.
//! - **Parameters** are bound as `INT8`, `FLOAT8` or `TEXT`. `NULL` is bound
//!   untyped, so Postgres infers its type from where it is used
//!   (`delivered_at = $1`); such statements aren't cached, so a type inferred
//!   from a `NULL` never sticks to a statement that later binds a value (one
//!   cached from an earlier run with values is reused, with the values'
//!   types). Where nothing determines the type (`$2 IS NULL OR id = $2`,
//!   since Postgres types `$2` at `IS NULL`), Postgres refuses the statement,
//!   and the operation is run again with that statement's `NULL`s as `TEXT`;
//!   the other statements of a batch keep theirs inferred. (So one statement
//!   can't bind both kinds of `NULL`.)
//! - **Rows** decode into the store's [`Value`]s: integers of every width and
//!   booleans as `Int`, `NUMERIC` (what `SUM` returns) as `Int` when integral,
//!   floats as `Real`, text as `Text`.
//! - **Transactions are serializable**, like SQLite's, so conditional writes
//!   (the entrant cap) stay exact under concurrency. Postgres aborts a
//!   conflicting transaction with a serialization failure; every operation
//!   here is self-contained, so it is retried from the start.

use std::{
    borrow::Cow,
    future::Future,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use sqlx::{
    Column, Connection, Encode, PgPool, Row as _, Type, TypeInfo, ValueRef,
    encode::IsNull,
    error::BoxDynError,
    postgres::{
        PgArgumentBuffer, PgArguments, PgConnectOptions, PgConnection, PgPoolOptions, PgRow,
        PgTypeInfo, PgValueFormat, PgValueRef, types::Oid,
    },
};

use crate::{
    error::{Error, Result},
    query::Statement,
    value::{Row, Value},
};

type Pg = sqlx::Postgres;

#[derive(Clone)]
pub(crate) struct Postgres {
    pool: PgPool,
}

/// How often an operation is tried when Postgres aborts it as a serialization
/// failure or deadlock.
const ATTEMPTS: u32 = 10;

fn options(url: &str) -> Result<PgConnectOptions> {
    Ok(PgConnectOptions::from_str(url)?
        .options([("default_transaction_isolation", "serializable")]))
}

/// Opens a pool on the existing database at `url`.
pub(super) async fn connect(url: &str) -> Result<Postgres> {
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .idle_timeout(Duration::from_secs(600))
        .max_lifetime(Duration::from_secs(3600))
        .connect_with(options(url)?)
        .await?;
    Ok(Postgres { pool })
}

/// Opens a pool lazily: the first query connects.
pub(super) fn connect_lazy(url: &str) -> Result<Postgres> {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy_with(options(url)?);
    Ok(Postgres { pool })
}

/// Creates a new, empty schema in the Postgres database at `url` and returns
/// a URL whose connections use it (through `search_path`): a fresh database
/// for each test, on one server. The schema is left behind; use a throwaway
/// database.
pub async fn fresh_schema(url: &str) -> Result<String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let schema = format!(
        "rafi_test_{}_{}_{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let mut conn = PgConnection::connect(url).await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&mut conn)
        .await?;
    conn.close().await?;
    let sep = if url.contains('?') { '&' } else { '?' };
    Ok(format!("{url}{sep}options=-c%20search_path%3D{schema}"))
}

/// Rewrites `?N` to `$N` and each bare `?` to the next `$N`, outside string
/// literals, quoted names and comments.
fn placeholders(sql: &str) -> Cow<'_, str> {
    if !sql.contains('?') {
        return Cow::Borrowed(sql);
    }
    let mut out = String::with_capacity(sql.len() + 8);
    let mut chars = sql.chars().peekable();
    let mut next = 0u32;
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                out.push(c);
                for d in chars.by_ref() {
                    out.push(d);
                    if d == c {
                        break;
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                out.push(c);
                for d in chars.by_ref() {
                    out.push(d);
                    if d == '\n' {
                        break;
                    }
                }
            }
            '?' => {
                out.push('$');
                if chars.peek().is_some_and(char::is_ascii_digit) {
                    while let Some(d) = chars.next_if(char::is_ascii_digit) {
                        out.push(d);
                    }
                } else {
                    next += 1;
                    out.push_str(&next.to_string());
                }
            }
            _ => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// How `NULL` parameters are typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Nulls {
    /// Untyped: Postgres infers the type from where the parameter is used.
    Inferred,
    /// `TEXT`, for a statement where nothing determines the type (Postgres
    /// reports an indeterminate type); what an untyped literal would be.
    Text,
}

/// A bound value: `INT8`, `FLOAT8`, `TEXT`, or a `NULL`.
struct Param<'a>(&'a Value, Nulls);

/// Postgres infers the type of a parameter declared with OID 0.
const UNTYPED: PgTypeInfo = PgTypeInfo::with_oid(Oid(0));

impl Type<Pg> for Param<'_> {
    fn type_info() -> PgTypeInfo {
        UNTYPED
    }
}

impl Encode<'_, Pg> for Param<'_> {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        match self.0 {
            Value::Null => Ok(IsNull::Yes),
            Value::Int(i) => <i64 as Encode<Pg>>::encode_by_ref(i, buf),
            Value::Real(f) => <f64 as Encode<Pg>>::encode_by_ref(f, buf),
            Value::Text(s) => <&str as Encode<Pg>>::encode_by_ref(&s.as_str(), buf),
        }
    }

    fn produces(&self) -> Option<PgTypeInfo> {
        Some(match self {
            Param(Value::Null, Nulls::Inferred) => UNTYPED,
            Param(Value::Int(_), _) => <i64 as Type<Pg>>::type_info(),
            Param(Value::Real(_), _) => <f64 as Type<Pg>>::type_info(),
            Param(Value::Text(_) | Value::Null, _) => <str as Type<Pg>>::type_info(),
        })
    }
}

/// Statements come from rafi's own code with every value bound as a parameter,
/// never interpolated, which is what sqlx's audit marker asks for.
fn prepare(stmt: &Statement, nulls: Nulls) -> sqlx::query::Query<'_, Pg, PgArguments> {
    let sql = placeholders(&stmt.sql).into_owned();
    stmt.params
        .iter()
        .fold(sqlx::query(sqlx::AssertSqlSafe(sql)), |q, p| {
            q.bind(Param(p, nulls))
        })
        .persistent(!has_null(stmt))
}

fn has_null(stmt: &Statement) -> bool {
    stmt.params.iter().any(|p| matches!(p, Value::Null))
}

fn to_row(row: &PgRow) -> Result<Row> {
    let mut out = Row::default();
    for (i, col) in row.columns().iter().enumerate() {
        out.columns.push(col.name().to_owned());
        let raw = row.try_get_raw(i)?;
        let value = if raw.is_null() {
            Value::Null
        } else {
            decode(raw).map_err(|e| Error::Decode(format!("column {}: {e}", col.name())))?
        };
        out.values.push(value);
    }
    Ok(out)
}

/// Decodes a non-NULL column value by its type's OID.
fn decode(raw: PgValueRef<'_>) -> Result<Value, BoxDynError> {
    let oid = raw.type_info().oid().map_or(0, |o| o.0);
    if raw.format() == PgValueFormat::Text {
        let s = raw.as_str()?;
        return Ok(match oid {
            16 => Value::Int((s == "t").into()),
            20 | 21 | 23 => Value::Int(s.parse()?),
            700 | 701 => Value::Real(s.parse()?),
            1700 => s
                .parse()
                .map(Value::Int)
                .or_else(|_| s.parse().map(Value::Real))?,
            _ => Value::Text(s.to_owned()),
        });
    }
    let b = raw.as_bytes()?;
    Ok(match oid {
        16 => Value::Int((b.first() == Some(&1)).into()),
        21 => Value::Int(i16::from_be_bytes(b.try_into()?).into()),
        23 => Value::Int(i32::from_be_bytes(b.try_into()?).into()),
        20 => Value::Int(i64::from_be_bytes(b.try_into()?)),
        700 => Value::Real(f32::from_be_bytes(b.try_into()?).into()),
        701 => Value::Real(f64::from_be_bytes(b.try_into()?)),
        1700 => numeric(b)?,
        // text, varchar, bpchar, name, unknown, json
        25 | 1043 | 1042 | 19 | 705 | 114 => Value::Text(std::str::from_utf8(b)?.to_owned()),
        // jsonb: a version byte, then the text
        3802 => Value::Text(std::str::from_utf8(b.get(1..).unwrap_or_default())?.to_owned()),
        _ => return Err(format!("unsupported type {}", raw.type_info().name()).into()),
    })
}

/// Decodes a binary `NUMERIC`: base-10000 digits with a weight (the power of
/// 10000 of the first digit). Integral values become `Int`, others `Real`.
fn numeric(b: &[u8]) -> Result<Value, BoxDynError> {
    let word = |i: usize| -> Result<u16, BoxDynError> {
        Ok(u16::from_be_bytes(
            b.get(i..i + 2).ok_or("truncated NUMERIC")?.try_into()?,
        ))
    };
    let ndigits = usize::from(word(0)?);
    let weight = i32::from(word(2)? as i16);
    let sign = word(4)?;
    if sign == 0xC000 {
        return Ok(Value::Real(f64::NAN));
    }
    let negative = sign == 0x4000;
    let digits = (0..ndigits)
        .map(|k| word(8 + 2 * k))
        .collect::<Result<Vec<_>, _>>()?;
    let integral = digits
        .iter()
        .enumerate()
        .all(|(k, &d)| d == 0 || weight - k as i32 >= 0);
    if integral {
        let mut n: i128 = 0;
        let mut ok = true;
        for (k, &d) in digits.iter().enumerate() {
            let power = 10_000i128.checked_pow((weight - k as i32) as u32);
            match power
                .and_then(|p| p.checked_mul(d.into()))
                .and_then(|v| n.checked_add(v))
            {
                Some(v) => n = v,
                None => ok = false,
            }
        }
        if ok && let Ok(n) = i64::try_from(if negative { -n } else { n }) {
            return Ok(Value::Int(n));
        }
    }
    let f: f64 = digits
        .iter()
        .enumerate()
        .map(|(k, &d)| f64::from(d) * 10_000f64.powi(weight - k as i32))
        .sum();
    Ok(Value::Real(if negative { -f } else { f }))
}

fn sqlstate(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(d) => d.code().map(Cow::into_owned),
        _ => None,
    }
}

/// A failed operation, and which of its statements failed, if one did.
struct Failed {
    stmt: Option<usize>,
    error: sqlx::Error,
}

impl From<sqlx::Error> for Failed {
    fn from(error: sqlx::Error) -> Self {
        Self { stmt: None, error }
    }
}

impl Failed {
    /// Blames statement `stmt`.
    fn at(stmt: usize) -> impl FnOnce(sqlx::Error) -> Self {
        move |error| Self {
            stmt: Some(stmt),
            error,
        }
    }
}

/// Runs `op`, made of `stmts`, until it succeeds or fails with something
/// other than a serialization failure or deadlock, at most [`ATTEMPTS`]
/// times. `op` gets how to type each statement's `NULL`s: if a statement
/// binds `NULL` where nothing determines its type (`?2 IS NULL` alone), the
/// operation is run again with that statement's `NULL`s typed as `TEXT`, and
/// the others' left as they were.
async fn retrying<T, F, Fut>(stmts: &[Statement], mut op: F) -> Result<T>
where
    F: FnMut(Vec<Nulls>) -> Fut,
    Fut: Future<Output = Result<T, Failed>>,
{
    let mut attempt = 1;
    let mut nulls = vec![Nulls::Inferred; stmts.len()];
    loop {
        let Failed { stmt, error } = match op(nulls.clone()).await {
            Ok(v) => return Ok(v),
            Err(failed) => failed,
        };
        match (sqlstate(&error).as_deref(), stmt) {
            // ambiguous_parameter ("could not determine data type of parameter"),
            // indeterminate_datatype
            (Some("42P08" | "42P18"), Some(i))
                if has_null(&stmts[i]) && nulls[i] == Nulls::Inferred =>
            {
                nulls[i] = Nulls::Text;
            }
            // serialization_failure, deadlock_detected
            (Some("40001" | "40P01"), _) if attempt < ATTEMPTS => {
                // A little jitter, so conflicting writers don't meet again.
                let jitter = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_micros() % 5_000);
                tokio::time::sleep(Duration::from_micros(
                    u64::from(attempt) * 2_000 + u64::from(jitter),
                ))
                .await;
                attempt += 1;
            }
            _ => return Err(error.into()),
        }
    }
}

impl Postgres {
    pub(crate) async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../migrations/postgres")
            .run(&self.pool)
            .await
            .map_err(|e| Error::Backend(e.to_string()))
    }

    pub(crate) async fn all(&self, stmt: Statement) -> Result<Vec<Row>> {
        let (pool, stmts) = (&self.pool, std::slice::from_ref(&stmt));
        retrying(stmts, |nulls| async move {
            prepare(&stmts[0], nulls[0])
                .fetch_all(pool)
                .await
                .map_err(Failed::at(0))
        })
        .await?
        .iter()
        .map(to_row)
        .collect()
    }

    pub(crate) async fn run(&self, stmt: Statement) -> Result<u64> {
        let (pool, stmts) = (&self.pool, std::slice::from_ref(&stmt));
        retrying(stmts, |nulls| async move {
            Ok(prepare(&stmts[0], nulls[0])
                .execute(pool)
                .await
                .map_err(Failed::at(0))?
                .rows_affected())
        })
        .await
    }

    pub(crate) async fn batch(&self, stmts: Vec<Statement>) -> Result<Vec<u64>> {
        let (pool, stmts) = (&self.pool, &stmts);
        retrying(stmts, |nulls| async move {
            let mut tx = pool.begin().await?;
            let mut changes = Vec::with_capacity(stmts.len());
            for (i, stmt) in stmts.iter().enumerate() {
                changes.push(
                    prepare(stmt, nulls[i])
                        .execute(&mut *tx)
                        .await
                        .map_err(Failed::at(i))?
                        .rows_affected(),
                );
            }
            tx.commit().await?;
            Ok(changes)
        })
        .await
    }

    pub(crate) async fn batch_rows(&self, stmts: Vec<Statement>) -> Result<Vec<Vec<Row>>> {
        let (pool, stmts) = (&self.pool, &stmts);
        let sets = retrying(stmts, |nulls| async move {
            let mut tx = pool.begin().await?;
            let mut sets = Vec::with_capacity(stmts.len());
            for (i, stmt) in stmts.iter().enumerate() {
                sets.push(
                    prepare(stmt, nulls[i])
                        .fetch_all(&mut *tx)
                        .await
                        .map_err(Failed::at(i))?,
                );
            }
            tx.commit().await?;
            Ok(sets)
        })
        .await?;
        sets.iter()
            .map(|rows| rows.iter().map(to_row).collect())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_placeholders() {
        assert_eq!(placeholders("SELECT 1"), "SELECT 1");
        assert_eq!(
            placeholders("UPDATE t SET a = ? WHERE id = ? AND b = ?"),
            "UPDATE t SET a = $1 WHERE id = $2 AND b = $3"
        );
        assert_eq!(
            placeholders("SELECT ?1, ?12 WHERE ?2 IS NULL OR id = ?2"),
            "SELECT $1, $12 WHERE $2 IS NULL OR id = $2"
        );
        assert_eq!(
            placeholders("SELECT '?', \"a?\", ? -- why?\n, ?"),
            "SELECT '?', \"a?\", $1 -- why?\n, $2"
        );
        assert_eq!(
            placeholders("json_extract(x, '$.a') = ?"),
            "json_extract(x, '$.a') = $1"
        );
    }

    fn numeric_bytes(weight: i16, sign: u16, dscale: u16, digits: &[u16]) -> Vec<u8> {
        let mut b = Vec::new();
        for w in [digits.len() as u16, weight as u16, sign, dscale] {
            b.extend_from_slice(&w.to_be_bytes());
        }
        for d in digits {
            b.extend_from_slice(&d.to_be_bytes());
        }
        b
    }

    #[test]
    fn decodes_numerics() {
        assert_eq!(
            numeric(&numeric_bytes(0, 0, 0, &[])).unwrap(),
            Value::Int(0)
        );
        assert_eq!(
            numeric(&numeric_bytes(0, 0, 0, &[3])).unwrap(),
            Value::Int(3)
        );
        // 12_345_678 = 1234 * 10000 + 5678
        assert_eq!(
            numeric(&numeric_bytes(1, 0, 0, &[1234, 5678])).unwrap(),
            Value::Int(12_345_678)
        );
        // 20000: one digit of weight 1 (trailing zero digits are dropped)
        assert_eq!(
            numeric(&numeric_bytes(1, 0x4000, 0, &[2])).unwrap(),
            Value::Int(-20_000)
        );
        // 1.5 = 1 + 5000 / 10000
        assert_eq!(
            numeric(&numeric_bytes(0, 0, 1, &[1, 5000])).unwrap(),
            Value::Real(1.5)
        );
    }
}
