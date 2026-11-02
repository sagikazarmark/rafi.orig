//! The `EXPLAIN QUERY PLAN` gate: the store's hot statements reach the rows
//! they need through an index, never by reading every row of a table that
//! grows with the installation (`entrants`, `raffles`, `raffle_events`).
//!
//! Each hot use case runs on an in-memory SQLite with the migrations, on
//! data that takes it down its whole path (the join inserts, housekeeping
//! has something to expire, erase and purge, the engine stores a claim), and
//! every statement it sent ([`capture`], a batch's every statement too) is
//! explained with its own parameters. On one of those tables:
//!
//! - `SCAN t` fails: every row;
//! - `SCAN t USING [COVERING] INDEX i` fails too: every entry of the index,
//!   as many as the table has rows; unless `i` is a partial index whose rows
//!   are bounded by what is left to do, not by the installation's age
//!   ([`BOUNDED`]);
//! - `SEARCH t USING … (constraint)` passes: a seek;
//! - `SEARCH t USING [COVERING] INDEX i` with no constraint fails (unless
//!   `i` is bounded): SQLite prints that for a `MIN`/`MAX` seek and for a
//!   walk of the whole index alike (`MIN(created_at)` over `raffles_owner`,
//!   whose first column is the owner, is the walk).
//!
//! A scan of anything else (a small table, a `VALUES` list, a constant row)
//! passes. The plans are SQLite's, and D1's (SQLite); Postgres plans by its
//! statistics. Rafi runs no `ANALYZE`, so the planner decides by the schema
//! alone, the same for a test's few rows as for an installation's many.

use rafi_raffle::{BeaconSource, Event, Manifest, ManifestItem, Raffle, Resolution};
use serde::Deserialize;

use crate::{
    Db, entrants, housekeeping, live, pow,
    query::{Statement, capture},
    raffles, retention, save_raffle, view,
};

/// The tables whose rows grow with every raffle or every entrant.
const LARGE: [&str; 3] = ["entrants", "raffles", "raffle_events"];

/// The partial indexes a whole walk of is fine, and why their rows are few.
const BOUNDED: [(&str, &str); 1] = [(
    "entrants_unerased",
    "entrants not erased yet: those of raffles before their erasure date, and winners held",
)];

/// Now, for every use case: past the ceiling of the raffles erased at 0,
/// before the erasure date of the ones that end in 2100.
const NOW: i64 = 10_000_000;

/// The winner hold.
const HOLD: i64 = 30 * 86_400;

/// When the raffles that are still on end.
const IN_2100: i64 = 4_102_444_800;

fn manifest(raffle: &str) -> Manifest {
    Manifest::new(
        raffle,
        BeaconSource::QUICKNET,
        vec![format!("a-{raffle}").into(), format!("b-{raffle}").into()],
        vec![
            ManifestItem {
                id: format!("{raffle}p1-1").into(),
                name: "Book".into(),
            },
            ManifestItem {
                id: format!("{raffle}p2-1").into(),
                name: "Shirt".into(),
            },
        ],
    )
}

/// Anna drawn for the prize handed over on the spot, and claimed.
fn claimed(raffle: &str) -> [Event; 2] {
    [
        Event::Drawn {
            k: 1,
            item: format!("{raffle}p2-1").into(),
            round: 1,
            signature: String::new(),
            randomness: String::new(),
            pass: 1,
            pool_size: 2,
            index: 0,
            entrant: format!("a-{raffle}").into(),
        },
        Event::Resolved {
            k: 1,
            resolution: Resolution::Claimed,
        },
    ]
}

/// A raffle in `state`, its event ending at `event_ends_at` and kept for a
/// week, with Anna (who joined) and Bence (added by the organizer) and two
/// prizes, the second handed over on the spot; with a manifest if `frozen`.
async fn raffle(db: &Db, raffle: &str, state: &str, event_ends_at: i64, frozen: bool) {
    let manifest = manifest(raffle);
    db.query(
        "INSERT INTO raffles (id, owner_id, slug, join_code, title, state, manifest, manifest_hash, event_date,
                              event_ends_at, retention_days, created_at, updated_at)
         VALUES (?1, 'o', 's-' || ?1, 'j-' || ?1, 't', ?2, ?3, ?4, '2100-01-01', ?5, 7, 0, 0)",
    )
    .bind(raffle)
    .bind(state)
    .bind(frozen.then(|| manifest.canonical_json()))
    .bind(frozen.then(|| manifest.hash().to_string()))
    .bind(event_ends_at)
    .run()
    .await
    .unwrap();
    for (n, (who, name, source)) in [("a-", "Anna", "self"), ("b-", "Bence", "manual")]
        .into_iter()
        .enumerate()
    {
        db.query(
            "INSERT INTO entrants (id, raffle_id, display_name, email_norm, claim_code, token_hash, created_at, source)
             VALUES (?1 || ?2, ?2, ?3, ?3 || '@example.com', 'X' || ?4, 't-' || ?2 || ?4, 0, ?5)",
        )
        .bind(who)
        .bind(raffle)
        .bind(name)
        .bind(n as i64)
        .bind(source)
        .run()
        .await
        .unwrap();
    }
    db.query(
        "INSERT INTO prizes (id, raffle_id, name, qty, position, delivery)
         VALUES (?1 || 'p1', ?1, 'Book', 1, 0, 'later'), (?1 || 'p2', ?1, 'Shirt', 1, 1, 'on_the_spot')",
    )
    .bind(raffle)
    .run()
    .await
    .unwrap();
}

/// The raffles the use cases find:
///
/// - `r1`: open, people join;
/// - `r2`: finished long ago, Anna's prize claimed, Bence objected, someone
///   removed at their request: past the ceiling;
/// - `r3`: open, past its erasure date: it expires;
/// - `r4`: drawing, past the ceiling: unfinished;
/// - `r5`: opened by the engine, which stores a claim.
async fn installation() -> Db {
    let db = Db::connect("sqlite::memory:").await.unwrap();
    raffle(&db, "r1", "open", IN_2100, false).await;
    raffle(&db, "r2", "drawing", 0, true).await;
    raffle(&db, "r3", "open", 0, false).await;
    raffle(&db, "r4", "drawing", 0, true).await;
    raffle(&db, "r5", "drawing", IN_2100, true).await;
    let r2 = Raffle::restore(&manifest("r2"), &claimed("r2")).unwrap();
    assert!(save_raffle(&db, &r2, 1).await.unwrap());
    let r5 = Raffle::restore(&manifest("r5"), &[]).unwrap();
    assert!(save_raffle(&db, &r5, 1).await.unwrap());
    for sql in [
        "UPDATE raffles SET state = 'finished', finished_at = 1 WHERE id = 'r2'",
        "UPDATE entrants SET objected_at = 1 WHERE raffle_id = 'r2' AND source = 'manual'",
        "INSERT INTO add_removals (raffle_id, email_norm, removed_at) VALUES ('r2', 'carla@example.com', 0)",
    ] {
        db.query(sql).run().await.unwrap();
    }
    db
}

/// The statements the use cases sent, by use case.
#[derive(Default)]
struct Sent(Vec<(&'static str, Statement)>);

impl Sent {
    /// Runs use case `name` and keeps the statements it sent.
    async fn run<T>(
        &mut self,
        name: &'static str,
        run: impl Future<Output = crate::Result<T>>,
    ) -> T {
        let (out, sent) = capture::statements(run).await;
        let out = out.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!sent.is_empty(), "{name} sent nothing");
        self.0.extend(sent.into_iter().map(|stmt| (name, stmt)));
        out
    }
}

#[derive(Deserialize)]
struct Line {
    detail: String,
}

/// What is wrong with a line of `sql`'s plan, if anything.
fn objection(sql: &str, line: &str) -> Option<String> {
    let words: Vec<&str> = line.split_whitespace().collect();
    let (op, name) = match words.as_slice() {
        [op @ ("SCAN" | "SEARCH"), name, ..] => (*op, *name),
        _ => return None,
    };
    let table = table_of(sql, name)?;
    let index = words
        .iter()
        .position(|w| *w == "INDEX")
        .and_then(|i| words.get(i + 1).copied());
    if let Some(index) = index
        && BOUNDED.iter().any(|(bounded, _)| *bounded == index)
    {
        return None;
    }
    match (op, index) {
        ("SEARCH", _) if line.ends_with(')') => None,
        ("SEARCH", _) => Some(format!("walks every entry of an index on {table}")),
        (_, None) => Some(format!("reads every row of {table}")),
        (_, Some(index)) => Some(format!("reads every entry of {index}, on {table}")),
    }
}

/// The large table `name` (a table or an alias) is in `sql`, if it is one.
fn table_of(sql: &str, name: &str) -> Option<&'static str> {
    if let Some(table) = LARGE.iter().find(|t| **t == name) {
        return Some(table);
    }
    let words: Vec<&str> = sql
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect();
    LARGE.into_iter().find(|table| {
        words.windows(2).any(|w| w == [*table, name])
            || words.windows(3).any(|w| w == [*table, "AS", name])
    })
}

/// Every statement of the hot use cases: how phones, screens and the admin
/// read a raffle, the join and its lookups, the engine's write and
/// housekeeping's sweeps. Each use case's result says the data took it down
/// the path whose statements are explained.
async fn hot_statements(db: &Db) -> Sent {
    let mut sent = Sent::default();
    // What the screens, the admin and phones read.
    assert!(
        sent.run("raffles::get", raffles::get(db, "r1"))
            .await
            .is_some()
    );
    assert!(
        sent.run("raffles::by_slug", raffles::by_slug(db, "s-r1"))
            .await
            .is_some()
    );
    let row = sent
        .run("raffles::by_join_code", raffles::by_join_code(db, "j-r5"))
        .await
        .unwrap();
    assert!(
        sent.run("raffles::drawn", raffles::drawn(db, &row))
            .await
            .is_some()
    );
    assert!(
        sent.run("view::Lite::read", view::Lite::read(db, "r1", NOW))
            .await
            .is_some()
    );
    assert!(
        sent.run("view::Full::read", view::Full::read(db, "r1"))
            .await
            .is_some()
    );
    assert!(
        sent.run("live::snapshot", live::snapshot(db, "r1"))
            .await
            .is_some()
    );
    // The join and its lookups.
    assert!(sent.run("pow::redeem", pow::redeem(db, "c", NOW)).await);
    assert!(
        sent.run("entrants::by_token", entrants::by_token(db, "r1", "t-r10"))
            .await
            .is_some()
    );
    assert_eq!(
        sent.run("entrants::display_names", entrants::display_names(db, "r1"))
            .await
            .len(),
        2
    );
    assert_eq!(
        sent.run("entrants::claim_codes", entrants::claim_codes(db, "r1"))
            .await
            .len(),
        2
    );
    let join = entrants::Join {
        id: "c-r1",
        raffle_id: "r1",
        display_name: "Carla",
        email_raw: "carla@example.com",
        email_norm: "carla@example.com",
        claim_code: "Y",
        token_hash: "t-c",
        consent_version: "v",
    };
    assert!(
        sent.run("entrants::join", entrants::join(db, &join, 500, NOW))
            .await
    );
    assert!(
        sent.run(
            "entrants::email_taken",
            entrants::email_taken(db, "r1", "carla@example.com")
        )
        .await
    );
    assert!(
        sent.run("entrants::get", entrants::get(db, "r1", "c-r1"))
            .await
            .is_some()
    );
    // The engine stores a claim of a prize handed over on the spot.
    let r5 = Raffle::restore(&manifest("r5"), &claimed("r5")).unwrap();
    assert!(sent.run("save_raffle", save_raffle(db, &r5, NOW)).await);
    // Housekeeping, in the order a run goes.
    assert_eq!(
        sent.run(
            "housekeeping::live_channels_due",
            housekeeping::live_channels_due(db, NOW)
        )
        .await,
        ["r2"]
    );
    assert_eq!(
        sent.run("retention::expire_due", retention::expire_due(db, NOW))
            .await,
        ["r3"]
    );
    assert_eq!(
        sent.run(
            "retention::erase_objected",
            retention::erase_objected(db, NOW)
        )
        .await
        .entrants,
        1
    );
    let erased = sent
        .run("retention::erase_due", retention::erase_due(db, NOW, HOLD))
        .await;
    assert_eq!(
        (erased.entrants, &erased.purged[..]),
        (3, &["r2".to_owned()][..])
    );
    assert_eq!(
        sent.run(
            "retention::forget_removals",
            retention::forget_removals(db, NOW)
        )
        .await,
        1
    );
    assert_eq!(
        sent.run(
            "retention::unfinished_at_ceiling",
            retention::unfinished_at_ceiling(db, NOW, HOLD, None)
        )
        .await,
        ["r4"]
    );
    sent.run(
        "retention::purge_unfinished",
        retention::purge_unfinished(db, "r4", NOW),
    )
    .await;
    sent
}

#[tokio::test]
async fn hot_statements_use_an_index() {
    let db = installation().await;
    let sent = hot_statements(&db).await;
    let mut failures = Vec::new();
    for (name, stmt) in sent.0 {
        let explain = Statement {
            sql: format!("EXPLAIN QUERY PLAN {}", stmt.sql),
            params: stmt.params,
        };
        let plan: Vec<String> = db
            .backend
            .all(explain)
            .await
            .unwrap_or_else(|e| panic!("{name}: explaining {}: {e}", stmt.sql))
            .into_iter()
            .map(|row| row.decode::<Line>().unwrap().detail)
            .collect();
        for line in &plan {
            if let Some(why) = objection(&stmt.sql, line) {
                let sql = stmt.sql.split_whitespace().collect::<Vec<_>>().join(" ");
                failures.push(format!(
                    "{name}: `{line}` {why}\n  {sql}\n  {}",
                    plan.join(" | ")
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn the_rule() {
    let sql = "SELECT * FROM entrants e JOIN raffles AS r ON r.id = e.raffle_id, prizes p";
    // Every row, or every entry of an index, of a large table: by its name
    // or an alias.
    for line in [
        "SCAN entrants",
        "SCAN e",
        "SCAN r USING INDEX raffles_erase_at",
        "SCAN raffles USING COVERING INDEX raffles_owner",
        "SEARCH raffles USING COVERING INDEX raffles_owner",
    ] {
        assert!(objection(sql, line).is_some(), "{line}");
    }
    // A seek; a bounded partial index; anything else.
    for line in [
        "SEARCH e USING INDEX entrants_raffle (raffle_id=?)",
        "SEARCH r EXISTS USING INDEX sqlite_autoindex_raffles_1 (id=?)",
        "SCAN entrants USING INDEX entrants_unerased",
        "SCAN p",
        "SCAN prizes",
        "SCAN CONSTANT ROW",
        "SCAN 2-ROW VALUES CLAUSE",
        "USE TEMP B-TREE FOR ORDER BY",
        "CORRELATED SCALAR SUBQUERY 2",
    ] {
        assert!(objection(sql, line).is_none(), "{line}");
    }
}
