//! Scenarios every backend must pass. Native tests run them against SQLite;
//! `crates/store-d1-check` runs them against D1 under `wrangler dev`.

use std::future::Future;

use rafi_core::{
    commit::{CommitDoc, UnitRef},
    state::DrawState,
};

use crate::{Db, PurgeAfter, save_draw_state};

const PURGE: PurgeAfter = PurgeAfter {
    nonwinners_secs: 30,
    winners_secs: 90,
};

type Outcome = Result<(), String>;

fn check(ok: bool, what: &str) -> Outcome {
    if ok { Ok(()) } else { Err(what.to_owned()) }
}

fn fail(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Runs every scenario on a fresh database from `fresh`, which must return an
/// empty, migrated database each time it is called.
pub async fn run<F, Fut>(fresh: F) -> Vec<(&'static str, Outcome)>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Db>,
{
    vec![
        (
            "keeps_the_newest_state",
            keeps_the_newest_state(&fresh().await).await,
        ),
        ("bumps_the_version", bumps_the_version(&fresh().await).await),
        (
            "batches_are_atomic",
            batches_are_atomic(&fresh().await).await,
        ),
        ("decodes_rows", decodes_rows(&fresh().await).await),
    ]
}

async fn frozen(db: &Db, id: &str) -> Result<DrawState, String> {
    let doc = CommitDoc::new(
        id,
        &rafi_core::drand::chains::QUICKNET_HASH,
        vec!["a".repeat(32), "b".repeat(32)],
        vec![UnitRef {
            unit_id: "u1".into(),
            item_id: "i1".into(),
            item_name: "Ticket".into(),
        }],
    );
    let state = DrawState::freeze(&doc);
    db.query(
        "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, engine, list_hash, created_at, updated_at)
         VALUES (?, 'o', ?, ?, 't', 'frozen', 'restate', ?, 0, 0)",
    )
    .bind(id)
    .bind(format!("slug-{id}"))
    .bind(format!("code-{id}"))
    .bind(&state.list_hash)
    .run()
    .await
    .map_err(fail)?;
    Ok(state)
}

async fn keeps_the_newest_state(db: &Db) -> Outcome {
    let mut state = frozen(db, "r1").await?;
    check(
        save_draw_state(db, &state, PURGE).await.map_err(fail)?,
        "first write",
    )?;
    check(
        !save_draw_state(db, &state, PURGE).await.map_err(fail)?,
        "a retry is a no-op",
    )?;

    state.finish().map_err(fail)?;
    check(
        save_draw_state(db, &state, PURGE).await.map_err(fail)?,
        "finished write",
    )?;
    let (lifecycle, finished_at, nonwinners, winners): (String, i64, i64, i64) = db
        .query("SELECT state, finished_at, purge_nonwinners_at, purge_winners_at FROM raffles WHERE id = 'r1'")
        .one()
        .await
        .map_err(fail)?;
    check(lifecycle == "finished", "finished state")?;
    check(nonwinners == finished_at + 30, "non-winner purge date")?;
    check(winners == finished_at + 90, "winner purge date")?;

    let mut other = state.clone();
    other.list_hash = "0".repeat(64);
    other.version += 1;
    check(
        !save_draw_state(db, &other, PURGE).await.map_err(fail)?,
        "a different entrant list",
    )
}

async fn version(db: &Db) -> Result<i64, String> {
    Ok(db
        .query("SELECT version FROM raffles WHERE id = 'r1'")
        .one::<(i64,)>()
        .await
        .map_err(fail)?
        .0)
}

async fn bumps_the_version(db: &Db) -> Outcome {
    frozen(db, "r1").await?;
    let v0 = version(db).await?;
    db.query("UPDATE raffles SET title = 'x' WHERE id = 'r1'")
        .run()
        .await
        .map_err(fail)?;
    let v1 = version(db).await?;
    check(v1 == v0 + 1, "a raffle update bumps once")?;
    for sql in [
        "INSERT INTO items (id, raffle_id, name) VALUES ('i', 'r1', 'Book')",
        "INSERT INTO entrants (id, raffle_id, display_name, claim_code, created_at) VALUES ('e', 'r1', 'Anna', 'K2P9', 0)",
        "UPDATE entrants SET hidden = 1 WHERE id = 'e'",
        "DELETE FROM items WHERE id = 'i'",
    ] {
        db.query(sql).run().await.map_err(fail)?;
    }
    check(
        version(db).await? == v1 + 4,
        "item and entrant writes bump once each",
    )
}

async fn batches_are_atomic(db: &Db) -> Outcome {
    frozen(db, "r1").await?;
    let res = db
        .batch(vec![
            db.query("UPDATE raffles SET title = 'changed' WHERE id = 'r1'"),
            db.query(
                "INSERT INTO raffles (id, owner_id, slug, join_code, title, created_at, updated_at)
                 VALUES ('r2', 'o', 'slug-r1', 'k', 't', 0, 0)",
            ),
        ])
        .await;
    match res {
        Err(e) => check(e.is_unique_violation(), "the error is a unique violation")?,
        Ok(_) => return Err("a batch with a failing statement succeeded".into()),
    }
    let (title,): (String,) = db
        .query("SELECT title FROM raffles WHERE id = 'r1'")
        .one()
        .await
        .map_err(fail)?;
    check(title == "t", "the first statement rolled back")?;
    let changes = db
        .batch(vec![
            db.query("UPDATE raffles SET title = 'a' WHERE id = 'r1'"),
            db.query("UPDATE raffles SET title = 'b' WHERE id = 'nope'"),
        ])
        .await
        .map_err(fail)?;
    // D1 counts rows changed by triggers too (here, the version bump), so only
    // zero versus non-zero is portable.
    check(
        changes[0] > 0 && changes[1] == 0,
        &format!("changed rows per statement: {changes:?}"),
    )
}

async fn decodes_rows(db: &Db) -> Outcome {
    #[derive(serde::Deserialize)]
    struct Entrant {
        id: String,
        hidden: bool,
        email_raw: Option<String>,
        created_at: i64,
    }
    frozen(db, "r1").await?;
    db.query(
        "INSERT INTO entrants (id, raffle_id, display_name, claim_code, hidden, created_at)
         VALUES ('e', 'r1', 'Anna', 'K2P9', 1, 1760000000)",
    )
    .run()
    .await
    .map_err(fail)?;
    let e: Entrant = db
        .query("SELECT * FROM entrants WHERE id = ?")
        .bind("e")
        .one()
        .await
        .map_err(fail)?;
    check(
        e.id == "e" && e.hidden && e.email_raw.is_none() && e.created_at == 1_760_000_000,
        "struct decode",
    )?;
    let missing: Option<Entrant> = db
        .query("SELECT * FROM entrants WHERE id = ?")
        .bind("x")
        .optional()
        .await
        .map_err(fail)?;
    check(missing.is_none(), "optional finds nothing")?;
    let (n,): (i64,) = db
        .query("SELECT COUNT(*) FROM entrants WHERE raffle_id = ?")
        .bind("r1")
        .one()
        .await
        .map_err(fail)?;
    check(n == 1, "tuple decode")
}
