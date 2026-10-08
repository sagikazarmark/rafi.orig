//! The `query.*` scenarios: what the store's SQL relies on from the query
//! layer, the same on every backend.

use serde::Deserialize;

use super::{Outcome, check, fail};
use crate::{Db, Error};

/// The largest integer every backend returns exactly: D1 hands numbers to
/// JavaScript as doubles.
const MAX_EXACT: i64 = (1 << 53) - 1;

/// A raffle `id` in the default state, with `slug-<id>` and `code-<id>`.
async fn raffle(db: &Db, id: &str) -> Outcome {
    db.query(
        "INSERT INTO raffles (id, owner_id, slug, join_code, title, created_at, updated_at)
         VALUES (?1, 'o', ?2, ?3, 't', 0, 0)",
    )
    .bind(id)
    .bind(format!("slug-{id}"))
    .bind(format!("code-{id}"))
    .run()
    .await
    .map(|_| ())
    .map_err(fail)
}

async fn entrant(db: &Db, raffle_id: &str, id: &str, created_at: i64) -> Outcome {
    db.query(
        "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at)
         VALUES (?1, ?2, ?1, 'K2P9', ?3)",
    )
    .bind(id)
    .bind(raffle_id)
    .bind(created_at)
    .run()
    .await
    .map(|_| ())
    .map_err(fail)
}

async fn item(db: &Db, raffle_id: &str, id: &str, qty: i64, position: i64) -> Outcome {
    db.query("INSERT INTO items (id, raffle_id, name, qty, position) VALUES (?, ?, ?, ?, ?)")
        .bind(id)
        .bind(raffle_id)
        .bind(format!("Item {id}"))
        .bind(qty)
        .bind(position)
        .run()
        .await
        .map(|_| ())
        .map_err(fail)
}

async fn version(db: &Db, raffle_id: &str) -> Result<i64, String> {
    Ok(db
        .query("SELECT version FROM raffles WHERE id = ?")
        .bind(raffle_id)
        .one::<(i64,)>()
        .await
        .map_err(fail)?
        .0)
}

async fn count(db: &Db, sql: &str) -> Result<i64, String> {
    Ok(db.query(sql).one::<(i64,)>().await.map_err(fail)?.0)
}

/// Every type rafi reads decodes the same everywhere: text (any characters),
/// integers up to 2^53, flags stored as 0/1 and comparisons as `bool`,
/// `NULL`s as `None`, reals, `COUNT`, and `SUM`/`AVG` (`NUMERIC` in
/// Postgres); structs by column name, tuples by position.
///
/// D1 returns rows as JavaScript objects, which put integer-like keys first
/// and keep one value per key: every selected column needs a distinct name
/// that isn't an integer (alias expressions such as `1` or `x = 1`), or
/// tuples decode out of order. A one-column row (`SELECT 1`) is fine.
pub(super) async fn decode_types(db: &Db) -> Outcome {
    #[derive(Deserialize)]
    struct Entrant {
        id: String,
        display_name: String,
        claim_code: String,
        hidden: bool,
        purged: bool,
        email_raw: Option<String>,
        email_norm: Option<String>,
        delivered_at: Option<i64>,
        consent_rules_at: Option<i64>,
        created_at: i64,
    }
    raffle(db, "r1").await?;
    let name = "Ádám \"O'Neil\" ?1 $1 🎟";
    db.query(
        "INSERT INTO entrants (id, raffle_id, display_name, email_raw, claim_code, hidden,
                               consent_rules_at, created_at)
         VALUES ('e', 'r1', ?, '', '007', 1, ?, ?)",
    )
    .bind(name)
    .bind(-5)
    .bind(MAX_EXACT)
    .run()
    .await
    .map_err(fail)?;
    let e: Entrant = db
        .query("SELECT * FROM entrants WHERE id = ?")
        .bind("e")
        .one()
        .await
        .map_err(fail)?;
    check(e.id == "e" && e.display_name == name, "text round-trips")?;
    check(e.claim_code == "007", "numeric-looking text stays text")?;
    check(e.hidden && !e.purged, "0/1 flags as bool")?;
    check(
        e.email_raw.as_deref() == Some("") && e.email_norm.is_none(),
        "an empty string is not NULL",
    )?;
    check(
        e.delivered_at.is_none() && e.consent_rules_at == Some(-5),
        "nullable integers",
    )?;
    check(e.created_at == MAX_EXACT, "2^53 - 1 exactly")?;

    // Missing rows.
    let missing: Option<(String,)> = db
        .query("SELECT id FROM entrants WHERE id = ?")
        .bind("x")
        .optional()
        .await
        .map_err(fail)?;
    check(missing.is_none(), "optional finds nothing")?;
    let none = db
        .query("SELECT id FROM entrants WHERE id = ?")
        .bind("x")
        .one::<(String,)>()
        .await;
    check(
        matches!(none, Err(Error::NotFound)),
        &format!("one finds nothing: {none:?}"),
    )?;
    let null_into_int = db
        .query("SELECT delivered_at FROM entrants WHERE id = 'e'")
        .one::<(i64,)>()
        .await;
    check(
        matches!(null_into_int, Err(Error::Decode(_))),
        &format!("NULL into a non-optional integer: {null_into_int:?}"),
    )?;

    // Expressions: comparisons, literals, arithmetic on parameters.
    let flags: (bool, bool, i64, i64) = db
        .query(
            "SELECT hidden = 1 AS h, purged = 1 AS p, 1 AS one, COUNT(*) AS n
             FROM entrants WHERE raffle_id = ? GROUP BY hidden, purged",
        )
        .bind("r1")
        .one()
        .await
        .map_err(fail)?;
    check(
        flags == (true, false, 1, 1),
        &format!("comparisons, a literal and a count: {flags:?}"),
    )?;
    let (real, doubled, plus_one): (f64, f64, i64) = db
        .query("SELECT 1.5 AS r, ?1 * 2 AS d, ?2 + 1 AS p")
        .bind(1.25)
        .bind(41)
        .one()
        .await
        .map_err(fail)?;
    check(
        real == 1.5 && doubled == 2.5 && plus_one == 42,
        &format!("reals and integers: {real}, {doubled}, {plus_one}"),
    )?;

    // Aggregates: `SUM` over a `BIGINT` is `NUMERIC` in Postgres.
    item(db, "r1", "i1", 2, 0).await?;
    item(db, "r1", "i2", 3, 1).await?;
    let sums: (i64, i64, i64, f64, Option<i64>, i64) = db
        .query(
            "SELECT SUM(qty) AS s, SUM(-qty) AS neg, SUM(qty * 1000000000000) AS big,
                    AVG(qty) AS avg,
                    (SELECT SUM(qty) FROM items WHERE raffle_id = 'none') AS missing,
                    (SELECT COALESCE(SUM(qty), 0) FROM items WHERE raffle_id = 'none') AS zero
             FROM items WHERE raffle_id = ?",
        )
        .bind("r1")
        .one()
        .await
        .map_err(fail)?;
    check(
        sums == (5, -5, 5_000_000_000_000, 2.5, None, 0),
        &format!("sums: {sums:?}"),
    )?;

    // Raw rows keep column names and order.
    let rows = db
        .query("SELECT id, hidden AS h, delivered_at AS d FROM entrants")
        .rows()
        .await
        .map_err(fail)?;
    check(
        rows.len() == 1 && rows[0].columns == ["id", "h", "d"],
        &format!("column names in order: {rows:?}"),
    )?;
    #[derive(Deserialize, PartialEq, Debug)]
    struct Renamed {
        d: Option<i64>,
        h: bool,
        id: String,
    }
    let renamed: Renamed = rows[0].clone().decode().map_err(fail)?;
    check(
        renamed
            == Renamed {
                d: None,
                h: true,
                id: "e".into(),
            },
        "a struct by alias",
    )
}

/// `NULL` parameters: where the column determines their type, and where
/// nothing does (`?N IS NULL`, `(?N IS NULL OR id = ?N)`), in either order
/// with values for the same statement, alone and in batches.
pub(super) async fn null_params(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    // Text and integer columns.
    db.query(
        "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, claim_code,
                               delivered_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("a")
    .bind("r1")
    .bind("Anna")
    .bind(None::<&str>)
    .bind(None::<String>)
    .bind("K2P9")
    .bind(None::<i64>)
    .bind(0)
    .run()
    .await
    .map_err(fail)?;
    entrant(db, "r1", "b", 0).await?;
    entrant(db, "r1", "c", 0).await?;

    let set_delivered = |id: &'static str, at: Option<i64>| {
        db.query("UPDATE entrants SET delivered_at = ?1 WHERE id = ?2")
            .bind(at)
            .bind(id)
    };
    let delivered = |id: &'static str| {
        db.query("SELECT delivered_at FROM entrants WHERE id = ?")
            .bind(id)
            .one::<(Option<i64>,)>()
    };
    // NULL, then a value, then NULL again: whatever type a backend inferred
    // or cached for the statement must not stick.
    for at in [None, Some(7), None, Some(9)] {
        set_delivered("a", at).run().await.map_err(fail)?;
        let (got,) = delivered("a").await.map_err(fail)?;
        check(got == at, &format!("delivered_at = {at:?}: read {got:?}"))?;
    }

    // Nothing types the parameter but `IS NULL`.
    let matching = |id: Option<&'static str>| {
        db.query("SELECT COUNT(*) FROM entrants WHERE raffle_id = ?1 AND (?2 IS NULL OR id = ?2)")
            .bind("r1")
            .bind(id)
            .one::<(i64,)>()
    };
    for (id, n) in [(None, 3), (Some("b"), 1), (None, 3), (Some("z"), 0)] {
        let (got,) = matching(id).await.map_err(fail)?;
        check(
            got == n,
            &format!("(?2 IS NULL OR id = ?2) with {id:?}: {got}"),
        )?;
    }
    for (value, is_null) in [(None, true), (Some("x"), false), (None, true)] {
        let (got,): (bool,) = db
            .query("SELECT ?1 IS NULL")
            .bind(value)
            .one()
            .await
            .map_err(fail)?;
        check(got == is_null, &format!("?1 IS NULL with {value:?}"))?;
    }
    let (fallback,): (String,) = db
        .query("SELECT COALESCE(?1, 'fallback')")
        .bind(None::<&str>)
        .one()
        .await
        .map_err(fail)?;
    check(fallback == "fallback", "COALESCE over a NULL parameter")?;
    // `= NULL` matches nothing.
    let (n,): (i64,) = db
        .query("SELECT COUNT(*) FROM entrants WHERE email_norm = ?")
        .bind(None::<&str>)
        .one()
        .await
        .map_err(fail)?;
    check(n == 0, "= NULL matches nothing")?;

    // In a batch, next to statements without NULLs, and next to a NULL that
    // only `IS NULL` types. (The column-typed NULL's statement is new here,
    // so no backend can have a type for it cached from a run with a value.)
    let changes = db
        .batch(vec![
            set_delivered("b", Some(3)),
            db.query("UPDATE entrants SET delivered_at = ?1, hidden = 0 WHERE id = ?2")
                .bind(None::<i64>)
                .bind("a"),
            db.query(
                "UPDATE entrants SET hidden = 1 WHERE raffle_id = ?1 AND (?2 IS NULL OR id = ?2)",
            )
            .bind("r1")
            .bind(None::<&str>),
        ])
        .await
        .map_err(fail)?;
    check(
        changes.iter().all(|&n| n > 0),
        &format!("each statement changed rows: {changes:?}"),
    )?;
    check(
        delivered("a").await.map_err(fail)?.0.is_none()
            && delivered("b").await.map_err(fail)?.0 == Some(3),
        "the batch's writes",
    )?;
    check(
        count(db, "SELECT COUNT(*) FROM entrants WHERE hidden = 1").await? == 3,
        "a NULL filter in a batch matches everyone",
    )?;
    let sets = db
        .batch_rows(vec![
            db.query("SELECT COUNT(*) FROM entrants WHERE (?1 IS NULL OR id = ?1)")
                .bind(None::<&str>),
            db.query("SELECT COUNT(*) FROM entrants WHERE (?1 IS NULL OR id = ?1)")
                .bind("c"),
        ])
        .await
        .map_err(fail)?;
    let counts: Vec<(i64,)> = sets
        .into_iter()
        .map(|mut rows| rows.remove(0).decode())
        .collect::<Result<_, _>>()
        .map_err(fail)?;
    check(
        counts == [(3,), (1,)],
        &format!("NULL and a value in one read batch: {counts:?}"),
    )
}

/// A batch applies all of its statements or none: a failure (unique or not,
/// in any statement) rolls back the earlier ones, trigger effects included.
/// Later statements see the earlier ones' writes.
pub(super) async fn batch_atomicity(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    let v0 = version(db, "r1").await?;
    let res = db
        .batch(vec![
            db.query("UPDATE raffles SET title = 'changed' WHERE id = 'r1'"),
            db.query("INSERT INTO items (id, raffle_id, name) VALUES ('i1', 'r1', 'Book')"),
            db.query(
                "INSERT INTO raffles (id, owner_id, slug, join_code, title, created_at, updated_at)
                 VALUES ('r2', 'o', 'slug-r1', 'k', 't', 0, 0)",
            ),
        ])
        .await;
    match res {
        Err(e) => check(e.is_unique_violation(), &format!("a unique violation: {e}"))?,
        Ok(_) => return Err("a batch with a failing statement succeeded".into()),
    }
    let (title,): (String,) = db
        .query("SELECT title FROM raffles WHERE id = 'r1'")
        .one()
        .await
        .map_err(fail)?;
    check(title == "t", "the update rolled back")?;
    check(
        count(db, "SELECT COUNT(*) FROM items").await? == 0,
        "the insert rolled back",
    )?;
    check(
        version(db, "r1").await? == v0,
        "the triggers' bumps rolled back",
    )?;

    // A failure that isn't a unique violation (NOT NULL) rolls back too.
    let res = db
        .batch(vec![
            db.query("INSERT INTO items (id, raffle_id, name) VALUES ('i2', 'r1', 'Mug')"),
            db.query("INSERT INTO items (id, raffle_id) VALUES ('i3', 'r1')"),
        ])
        .await;
    check(
        res.as_ref().is_err_and(|e| !e.is_unique_violation()),
        &format!("a NOT NULL violation: {res:?}"),
    )?;
    check(
        count(db, "SELECT COUNT(*) FROM items").await? == 0,
        "nothing was inserted",
    )?;

    // A batch that succeeds applies everything, in order.
    db.batch(vec![
        db.query("INSERT INTO items (id, raffle_id, name) VALUES ('i4', 'r1', 'Hat')"),
        db.query("UPDATE items SET qty = qty + 3 WHERE id = 'i4'"),
        db.query(
            "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at)
             SELECT 'e', 'r1', name, 'K2P9', qty FROM items WHERE id = 'i4'",
        ),
    ])
    .await
    .map_err(fail)?;
    let (name, created_at): (String, i64) = db
        .query("SELECT display_name, created_at FROM entrants WHERE id = 'e'")
        .one()
        .await
        .map_err(fail)?;
    check(
        name == "Hat" && created_at == 4,
        "each statement saw the ones before it",
    )?;
    check(
        version(db, "r1").await? == v0 + 3,
        "and the triggers fired for each",
    )
}

/// `batch_rows` returns one row set per statement, in order, empty ones
/// included, and fails as a whole if one statement does.
pub(super) async fn batch_rows_snapshot(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    item(db, "r1", "i", 3, 0).await?;
    let mut sets = db
        .batch_rows(vec![
            db.query("SELECT version, title FROM raffles WHERE id = ?")
                .bind("r1"),
            db.query("SELECT SUM(qty) FROM items WHERE raffle_id = ?")
                .bind("r1"),
            db.query("SELECT id FROM entrants WHERE raffle_id = ?")
                .bind("r1"),
            db.query("SELECT id FROM items WHERE raffle_id = ? ORDER BY id")
                .bind("r1"),
        ])
        .await
        .map_err(fail)?;
    check(sets.len() == 4, "one row set per statement")?;
    check(sets[2].is_empty(), "an empty result")?;
    let (id,): (String,) = sets[3].remove(0).decode().map_err(fail)?;
    check(id == "i", "the last set")?;
    let (qty,): (i64,) = sets[1].remove(0).decode().map_err(fail)?;
    check(qty == 3, "an aggregate")?;
    let (v, title): (i64, String) = sets[0].remove(0).decode().map_err(fail)?;
    check(v > 0 && title == "t", "the raffle row")?;

    let broken = db
        .batch_rows(vec![
            db.query("SELECT id FROM raffles"),
            db.query("SELECT nope FROM raffles"),
        ])
        .await;
    check(broken.is_err(), "a failing statement fails the batch")
}

/// `batch_rows` reads one snapshot while another session writes: an item
/// count and the title written in the same batch always agree.
pub(super) async fn batch_rows_under_writes(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    db.query("UPDATE raffles SET title = '0' WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    const WRITES: usize = 25;
    let writer = async {
        for n in 1..=WRITES {
            db.batch(vec![
                db.query("INSERT INTO items (id, raffle_id, name) VALUES (?, 'r1', 'x')")
                    .bind(format!("i{n:03}")),
                db.query("UPDATE raffles SET title = ? WHERE id = 'r1'")
                    .bind(n.to_string()),
            ])
            .await
            .map_err(fail)?;
        }
        Ok::<_, String>(())
    };
    let reader = async {
        let mut reads = 0;
        loop {
            let sets = db
                .batch_rows(vec![
                    db.query("SELECT COUNT(*) FROM items WHERE raffle_id = 'r1'"),
                    db.query("SELECT title FROM raffles WHERE id = 'r1'"),
                ])
                .await
                .map_err(fail)?;
            let [items, title] = <[Vec<crate::Row>; 2]>::try_from(sets)
                .map_err(|s| format!("{} row sets", s.len()))?;
            let (items,): (i64,) = items[0].clone().decode().map_err(fail)?;
            let (title,): (String,) = title[0].clone().decode().map_err(fail)?;
            check(
                title == items.to_string(),
                &format!("read {items} items but title {title:?}"),
            )?;
            reads += 1;
            if items == WRITES as i64 {
                return Ok::<_, String>(reads);
            }
            // The writer failed (its error is reported) or never got a turn.
            check(reads < 10_000, "the writer stalled")?;
        }
    };
    let (written, read) = futures_util::future::join(writer, reader).await;
    written?;
    read.map(|_| ())
}

/// Unique and primary-key violations are [`Error::Unique`], from `run` and
/// from batches; other constraint failures and bad SQL are not. `NULL`s
/// don't collide, and `ON CONFLICT DO NOTHING` doesn't fail.
pub(super) async fn unique_violation(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    raffle(db, "r2").await?;
    let insert_raffle = |id: &'static str, slug: &'static str, code: &'static str| {
        db.query(
            "INSERT INTO raffles (id, owner_id, slug, join_code, title, created_at, updated_at)
             VALUES (?, 'o', ?, ?, 't', 0, 0)",
        )
        .bind(id)
        .bind(slug)
        .bind(code)
    };
    for (what, q) in [
        ("a primary key", insert_raffle("r1", "s9", "c9")),
        ("a UNIQUE column", insert_raffle("r9", "slug-r1", "c9")),
        (
            "another UNIQUE column",
            insert_raffle("r9", "s9", "code-r2"),
        ),
    ] {
        let res = q.run().await;
        check(
            res.as_ref().is_err_and(Error::is_unique_violation),
            &format!("{what}: {res:?}"),
        )?;
    }

    let with_email = |raffle_id: &'static str, id: &'static str, email: Option<&'static str>| {
        db.query(
            "INSERT INTO entrants (id, raffle_id, display_name, email_norm, claim_code, created_at)
             VALUES (?, ?, 'x', ?, 'K2P9', 0)",
        )
        .bind(id)
        .bind(raffle_id)
        .bind(email)
    };
    with_email("r1", "a", Some("a@x"))
        .run()
        .await
        .map_err(fail)?;
    let res = with_email("r1", "b", Some("a@x")).run().await;
    check(
        res.as_ref().is_err_and(Error::is_unique_violation),
        &format!("a composite UNIQUE: {res:?}"),
    )?;
    with_email("r2", "c", Some("a@x"))
        .run()
        .await
        .map_err(|e| format!("the same email in another raffle: {e}"))?;
    for id in ["d", "e"] {
        with_email("r1", id, None)
            .run()
            .await
            .map_err(|e| format!("NULLs don't collide: {e}"))?;
    }
    let res = db
        .batch(vec![
            db.query("INSERT INTO altcha_used (challenge, expires_at) VALUES ('c', 1)"),
            db.query("INSERT INTO altcha_used (challenge, expires_at) VALUES ('c', 2)"),
        ])
        .await;
    check(
        res.as_ref().is_err_and(Error::is_unique_violation),
        &format!("within a batch: {res:?}"),
    )?;

    for (what, sql) in [
        (
            "NOT NULL",
            "INSERT INTO items (id, raffle_id) VALUES ('i', 'r1')",
        ),
        (
            "a foreign key",
            "INSERT INTO items (id, raffle_id, name) VALUES ('i', 'nope', 'Book')",
        ),
        ("bad SQL", "INSERT INTO nowhere (id) VALUES ('i')"),
    ] {
        let res = db.query(sql).run().await;
        check(
            res.as_ref().is_err_and(|e| !e.is_unique_violation()),
            &format!("{what} is an error, not a unique violation: {res:?}"),
        )?;
    }
    let res = db
        .query(
            "INSERT INTO altcha_used (challenge, expires_at) VALUES ('c', 1)
             ON CONFLICT (challenge) DO NOTHING",
        )
        .run()
        .await;
    check(res.is_ok(), &format!("ON CONFLICT DO NOTHING: {res:?}"))
}

/// Change counts: zero when nothing matched (even on tables with triggers),
/// non-zero otherwise. Only zero versus non-zero is portable: D1 also counts
/// the rows triggers changed.
pub(super) async fn change_counts(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    item(db, "r1", "i", 1, 0).await?;
    let run = |sql: &'static str| async move { db.query(sql).run().await.map_err(fail) };
    for (sql, changes) in [
        ("UPDATE raffles SET title = 'x' WHERE id = 'r1'", true),
        ("UPDATE raffles SET title = title WHERE id = 'r1'", true),
        ("UPDATE raffles SET title = 'x' WHERE id = 'nope'", false),
        ("UPDATE items SET qty = 2 WHERE id = 'i'", true),
        ("UPDATE items SET qty = 2 WHERE id = 'nope'", false),
        (
            "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at)
             SELECT 'e', 'r1', 'x', 'K2P9', 0 WHERE 1 = 0",
            false,
        ),
        (
            "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at)
             SELECT 'e', 'r1', 'x', 'K2P9', 0 WHERE 1 = 1",
            true,
        ),
        (
            "INSERT INTO altcha_used (challenge, expires_at) VALUES ('c', 1)
             ON CONFLICT (challenge) DO NOTHING",
            true,
        ),
        (
            "INSERT INTO altcha_used (challenge, expires_at) VALUES ('c', 1)
             ON CONFLICT (challenge) DO NOTHING",
            false,
        ),
        ("DELETE FROM entrants WHERE id = 'nope'", false),
        ("DELETE FROM entrants WHERE id = 'e'", true),
    ] {
        let n = run(sql).await?;
        check((n > 0) == changes, &format!("{n} changes from {sql}"))?;
    }
    let changes = db
        .batch(vec![
            db.query("UPDATE items SET qty = 3 WHERE id = 'i'"),
            db.query("UPDATE items SET qty = 3 WHERE id = 'nope'"),
            db.query("DELETE FROM altcha_used WHERE challenge = 'nope'"),
            db.query("DELETE FROM items WHERE id = 'i'"),
        ])
        .await
        .map_err(fail)?;
    check(
        changes.len() == 4
            && changes[0] > 0
            && changes[1] == 0
            && changes[2] == 0
            && changes[3] > 0,
        &format!("changes per statement in a batch: {changes:?}"),
    )
}

/// `ORDER BY` ties break by the next key, and IDs (lowercase letters and
/// digits, as rafi makes them) sort the same everywhere. `NULL`s sort first
/// in SQLite and last in Postgres, so the store never orders by a nullable
/// column without `NULLS FIRST`/`NULLS LAST`, which every backend accepts.
pub(super) async fn ordering_ties(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    for (id, position) in [("c", 1), ("b", 0), ("a", 1), ("d", 0)] {
        item(db, "r1", id, 1, position).await?;
    }
    let ids = |sql: &'static str| async move {
        db.query(sql)
            .all::<(String,)>()
            .await
            .map(|rows| rows.into_iter().map(|(id,)| id).collect::<Vec<_>>())
            .map_err(fail)
    };
    let asc = ids("SELECT id FROM items WHERE raffle_id = 'r1' ORDER BY position, id").await?;
    check(
        asc == ["b", "d", "a", "c"],
        &format!("by position, then ID: {asc:?}"),
    )?;
    let desc =
        ids("SELECT id FROM items WHERE raffle_id = 'r1' ORDER BY position DESC, id DESC").await?;
    check(
        desc == ["c", "a", "d", "b"],
        &format!("descending: {desc:?}"),
    )?;

    for (id, created_at) in [
        ("f2", 5),
        ("a10", 5),
        ("f10", 5),
        ("a9", 5),
        ("z", 1),
        ("0f", 5),
    ] {
        entrant(db, "r1", id, created_at).await?;
    }
    let entered =
        ids("SELECT id FROM entrants WHERE raffle_id = 'r1' ORDER BY created_at, id").await?;
    check(
        entered == ["z", "0f", "a10", "a9", "f10", "f2"],
        &format!("by time, then ID as text: {entered:?}"),
    )?;

    for (id, at) in [("a9", 3), ("f2", 1)] {
        db.query("UPDATE entrants SET delivered_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .run()
            .await
            .map_err(fail)?;
    }
    let first = ids(
        "SELECT id FROM entrants WHERE raffle_id = 'r1' AND id IN ('a9', 'f2', 'z')
         ORDER BY delivered_at NULLS FIRST, id",
    )
    .await?;
    check(
        first == ["z", "f2", "a9"],
        &format!("NULLS FIRST: {first:?}"),
    )?;
    let last = ids(
        "SELECT id FROM entrants WHERE raffle_id = 'r1' AND id IN ('a9', 'f2', 'z')
         ORDER BY delivered_at DESC NULLS LAST, id",
    )
    .await?;
    check(last == ["a9", "f2", "z"], &format!("NULLS LAST: {last:?}"))
}

/// A statement with 100 bound parameters, D1's limit: a multi-row `INSERT`
/// numbered up to `?100`, and an `IN` list of 100 bare `?`.
pub(super) async fn param_limit(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    // 33 rows of three parameters each (?1–?99), and the raffle as ?100.
    let rows: Vec<String> = (0..33)
        .map(|r| {
            let p = 3 * r + 1;
            format!("(?{p}, ?100, ?{}, 'K2P9', ?{})", p + 1, p + 2)
        })
        .collect();
    let insert = db.query(format!(
        "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at) VALUES {}",
        rows.join(", ")
    ));
    let insert = (0..33)
        .fold(insert, |q, r| {
            q.bind(format!("e{r:02}"))
                .bind(format!("Entrant {r}"))
                .bind(r)
        })
        .bind("r1");
    let n = insert.run().await.map_err(fail)?;
    check(n > 0, "the 100-parameter insert")?;

    let list = vec!["?"; 100].join(", ");
    let select = db.query(format!(
        "SELECT COUNT(*), MAX(created_at) FROM entrants WHERE id IN ({list})"
    ));
    let (found, latest): (i64, i64) = (0..100)
        .fold(select, |q, r| q.bind(format!("e{r:02}")))
        .one()
        .await
        .map_err(fail)?;
    check(
        found == 33 && latest == 32,
        &format!("100 parameters in IN: {found} rows, latest {latest}"),
    )
}

/// The version triggers: every write to a raffle, its items or its entrants
/// bumps `raffles.version` once; a write that sets the version itself keeps
/// it; a write that matches nothing bumps nothing.
pub(super) async fn version_triggers(db: &Db) -> Outcome {
    raffle(db, "r1").await?;
    raffle(db, "r2").await?;
    let v0 = version(db, "r1").await?;
    db.query("UPDATE raffles SET title = 'x' WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    let v1 = version(db, "r1").await?;
    check(v1 == v0 + 1, "a raffle update bumps once")?;
    for sql in [
        "INSERT INTO items (id, raffle_id, name) VALUES ('i', 'r1', 'Book')",
        "UPDATE items SET qty = 2 WHERE id = 'i'",
        "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at) VALUES ('e', 'r1', 'Anna', 'K2P9', 0)",
        "UPDATE entrants SET hidden = 1 WHERE id = 'e'",
        "DELETE FROM items WHERE id = 'i'",
        "DELETE FROM entrants WHERE id = 'e'",
    ] {
        db.query(sql).run().await.map_err(fail)?;
    }
    let v2 = version(db, "r1").await?;
    check(
        v2 == v1 + 6,
        &format!("item and entrant writes bump once each: {v1} -> {v2}"),
    )?;
    check(
        version(db, "r2").await? == 0,
        "another raffle's version stays",
    )?;

    db.query("UPDATE raffles SET version = 100 WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    check(
        version(db, "r1").await? == 100,
        "a write that sets the version keeps it",
    )?;
    for sql in [
        "UPDATE raffles SET title = 'y' WHERE id = 'nope'",
        "UPDATE items SET qty = 3 WHERE id = 'nope'",
        "DELETE FROM entrants WHERE id = 'nope'",
    ] {
        db.query(sql).run().await.map_err(fail)?;
    }
    check(
        version(db, "r1").await? == 100,
        "writes that match nothing bump nothing",
    )?;
    db.batch(vec![
        db.query("UPDATE raffles SET title = 'a' WHERE id = 'r1'"),
        db.query("UPDATE raffles SET title = 'b' WHERE id = 'r1'"),
    ])
    .await
    .map_err(fail)?;
    check(
        version(db, "r1").await? == 102,
        "each statement in a batch bumps",
    )
}
