//! A raffle's entrants, and erasing their personal data.

use rafi_lifecycle::{
    RaffleState,
    entrant::{Origin, Source},
};
use rafi_raffle::Outcome;
use serde::Deserialize;

use crate::{Db, Result};

/// A row of `entrants`.
#[derive(Debug, Clone, Deserialize)]
pub struct EntrantRow {
    pub id: String,
    pub raffle_id: String,
    pub display_name: String,
    pub email_raw: Option<String>,
    pub email_norm: Option<String>,
    pub source: Source,
    pub hidden: bool,
    /// `None` once their personal data is erased.
    pub claim_code: Option<String>,
    pub token_hash: Option<String>,
    pub consent_version: Option<String>,
    pub consent_rules_at: Option<i64>,
    pub delivered_at: Option<i64>,
    /// Their personal data was erased (the `ERASED` write): the row stays, anonymous.
    pub erased: bool,
    pub created_at: i64,
    /// The organizer's add they came in with ([`insert_added`]); `None` for
    /// joins and fakes.
    pub batch_id: Option<String>,
    /// Their row in that add, if it was a list.
    pub batch_row: Option<i64>,
    /// Their ID in the organizer's registration system, if the add gave one.
    pub list_ref: Option<String>,
}

/// The order entrants are listed in: by when they entered, and an add's
/// rows, entered at one time, in the add's order. `COALESCE` because SQLite
/// sorts `NULL` first and Postgres last; then the ID, so ties come out the
/// same on every backend.
pub(crate) const ORDER: &str = "ORDER BY created_at, COALESCE(batch_row, 0), id";

/// The `SET` clause that erases an entrant's personal data: their name, email,
/// registration ID, cookie token and claim code. The anonymous ID and the draw
/// log stay, and the add they came in with (an ID and a row number).
pub(crate) const ERASED: &str = "display_name = '#' || substr(id, 1, 6), email_raw = NULL, \
     email_norm = NULL, list_ref = NULL, token_hash = NULL, claim_code = NULL, erased = 1";

/// Rows per `INSERT` in [`insert_fakes`]: 3 shared parameters plus 3 per row
/// stays within D1's 100 bound parameters per statement.
const FAKES_PER_INSERT: usize = (MAX_PARAMS - 3) / 3;

/// Rows per `INSERT` in [`insert_added`]: 4 shared parameters plus 7 per row
/// (13 rows, 95 parameters).
const ADDED_PER_INSERT: usize = (MAX_PARAMS - 4) / 7;

/// D1 binds at most 100 parameters per statement.
const MAX_PARAMS: usize = 100;

const _: () =
    assert!(3 + 3 * FAKES_PER_INSERT <= MAX_PARAMS && 4 + 7 * ADDED_PER_INSERT <= MAX_PARAMS);

/// A raffle's entrants, in the order they entered.
pub async fn list(db: &Db, raffle_id: &str) -> Result<Vec<EntrantRow>> {
    db.query(format!(
        "SELECT * FROM entrants WHERE raffle_id = ? {ORDER}"
    ))
    .bind(raffle_id)
    .all()
    .await
}

pub async fn get(db: &Db, raffle_id: &str, id: &str) -> Result<Option<EntrantRow>> {
    db.query("SELECT * FROM entrants WHERE raffle_id = ? AND id = ?")
        .bind(raffle_id)
        .bind(id)
        .optional()
        .await
}

/// The entrant whose cookie token hashes to `token_hash`.
pub async fn by_token(db: &Db, raffle_id: &str, token_hash: &str) -> Result<Option<EntrantRow>> {
    db.query("SELECT * FROM entrants WHERE raffle_id = ? AND token_hash = ?")
        .bind(raffle_id)
        .bind(token_hash)
        .optional()
        .await
}

/// How many entrants haven't been erased.
pub async fn count_active(db: &Db, raffle_id: &str) -> Result<i64> {
    let (n,): (i64,) = db
        .query("SELECT COUNT(*) FROM entrants WHERE raffle_id = ? AND erased = 0")
        .bind(raffle_id)
        .one()
        .await?;
    Ok(n)
}

/// The IDs of the entrants that haven't been erased: the list a freeze commits.
pub async fn active_ids(db: &Db, raffle_id: &str) -> Result<Vec<String>> {
    let ids: Vec<(String,)> = db
        .query("SELECT id FROM entrants WHERE raffle_id = ? AND erased = 0")
        .bind(raffle_id)
        .all()
        .await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
}

/// How many fake entrants the raffle has.
pub async fn count_fakes(db: &Db, raffle_id: &str) -> Result<i64> {
    let (n,): (i64,) = db
        .query(format!(
            "SELECT COUNT(*) FROM entrants WHERE raffle_id = ? AND source = '{}'",
            Source::Fake.as_str()
        ))
        .bind(raffle_id)
        .one()
        .await?;
    Ok(n)
}

/// Every claim code in use in the raffle.
pub async fn claim_codes(db: &Db, raffle_id: &str) -> Result<Vec<String>> {
    let codes: Vec<(String,)> = db
        .query("SELECT claim_code FROM entrants WHERE raffle_id = ? AND claim_code IS NOT NULL")
        .bind(raffle_id)
        .all()
        .await?;
    Ok(codes.into_iter().map(|(c,)| c).collect())
}

/// Every display name in the raffle, erased entrants' placeholders included.
pub async fn display_names(db: &Db, raffle_id: &str) -> Result<Vec<String>> {
    let names: Vec<(String,)> = db
        .query("SELECT display_name FROM entrants WHERE raffle_id = ?")
        .bind(raffle_id)
        .all()
        .await?;
    Ok(names.into_iter().map(|(n,)| n).collect())
}

/// Whether someone with this normalized email has entered.
pub async fn email_taken(db: &Db, raffle_id: &str, email_norm: &str) -> Result<bool> {
    let found: Option<(i64,)> = db
        .query("SELECT 1 FROM entrants WHERE raffle_id = ? AND email_norm = ?")
        .bind(raffle_id)
        .bind(email_norm)
        .optional()
        .await?;
    Ok(found.is_some())
}

/// A self-service entry from the participant page.
#[derive(Debug, Clone)]
pub struct Join<'a> {
    pub id: &'a str,
    pub raffle_id: &'a str,
    pub display_name: &'a str,
    pub email_raw: &'a str,
    pub email_norm: &'a str,
    pub claim_code: &'a str,
    pub token_hash: &'a str,
    pub consent_version: &'a str,
}

/// Enters someone, only while registration is open and the raffle has fewer
/// than `max_entrants`: the hard limit behind the (soft) rate limits.
///
/// Returns `false` if registration isn't open or the raffle is full. An email
/// that already entered is an [`Error::Unique`](crate::Error::Unique). Past
/// the raffle's erasure date nobody joins: the raffle expires within the
/// hour ([`crate::retention::expire_due`]). The entry is made `now`, which is
/// also what the erasure date is compared with.
pub async fn join(db: &Db, j: &Join<'_>, max_entrants: i64, now: i64) -> Result<bool> {
    let inserted = db
        .query(format!(
            "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, source, claim_code, token_hash,
                                   consent_version, consent_rules_at, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, '{source}', ?6, ?7, ?8, ?9, ?10
             WHERE EXISTS (SELECT 1 FROM raffles WHERE id = ?2 AND state IN ({joinable}) AND erase_at > ?10)
               AND (SELECT COUNT(*) FROM entrants WHERE raffle_id = ?2) < ?11",
            source = Source::SelfService.as_str(),
            joinable = crate::raffles::states(RaffleState::joinable),
        ))
        .bind(j.id)
        .bind(j.raffle_id)
        .bind(j.display_name)
        .bind(j.email_raw)
        .bind(j.email_norm)
        .bind(j.claim_code)
        .bind(j.token_hash)
        .bind(j.consent_version)
        .bind(now)
        .bind(now)
        .bind(max_entrants)
        .run()
        .await?;
    Ok(inserted > 0)
}

/// The SQL condition that raffle `?{param}`'s entrants can still change
/// ([`RaffleState::entrants_editable`]): writes to the list check it in the
/// same statement, so none lands after the freeze.
fn editable(param: u8) -> String {
    format!(
        "EXISTS (SELECT 1 FROM raffles WHERE id = ?{param} AND state IN ({}))",
        crate::raffles::states(RaffleState::entrants_editable)
    )
}

/// The key of raffle `raffle_id`'s row in `batch_guards`: one per raffle, so
/// two adds into it (people or fakes) serialize, and adds into different
/// raffles don't wait for each other.
fn guard_key(raffle_id: &str) -> String {
    format!("add:{raffle_id}")
}

/// The condition each statement of a guarded batch writes under: the guard
/// row `?1` is there, so its first statement decided for all of them.
const GUARDED: &str = "EXISTS (SELECT 1 FROM batch_guards WHERE key = ?1)";

/// The guard's condition that raffle `?2` has room for `?3` more entrants
/// under the cap `?4`: counted inside the batch, so a join or another add
/// that landed first counts too.
const ROOM: &str = "(SELECT COUNT(*) FROM entrants WHERE raffle_id = ?2) + ?3 <= ?4";

/// A guarded batch's last statement: the guard row `key` goes, and
/// `batch_guards` is empty again.
fn release<'a>(db: &'a Db, key: &str) -> crate::Query<'a> {
    db.query("DELETE FROM batch_guards WHERE key = ?1")
        .bind(key.to_owned())
}

/// Someone the organizer adds ([`insert_added`]): one row of an add.
#[derive(Debug, Clone)]
pub struct Added<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub email_raw: Option<&'a str>,
    pub email_norm: Option<&'a str>,
    pub claim_code: &'a str,
    /// Their row in a list (`batch_row`), which keeps its order; `None`
    /// from the one-person form.
    pub row: Option<i64>,
    pub list_ref: Option<&'a str>,
}

/// An add: people the organizer adds at once ([`insert_added`]).
#[derive(Debug, Clone)]
pub struct AddBatch<'a> {
    pub raffle_id: &'a str,
    /// The add's ID, from the browser: a second add with it is refused.
    pub batch_id: &'a str,
    /// Where the add comes from, which decides the states and test mode the
    /// raffle takes it in ([`Origin::admits`]).
    pub origin: Origin,
    /// When the rows entered: the projector shows names from 5 s after.
    pub created_at: i64,
    pub rows: &'a [Added<'a>],
    /// The most entrants the raffle may have, the add included.
    pub max_entrants: i64,
    /// What the erasure date is compared with.
    pub now: i64,
}

/// Adds people the organizer vouches for ([`Source::Manual`]), all or
/// nothing, in one batch whose first statement decides for the whole add:
/// it inserts the raffle's guard row (`batch_guards`) only if the raffle
/// takes the add from its origin ([`Origin::admits`], not past its erasure
/// date), has room for every row under `max_entrants`, and has no entrant
/// from this batch ID yet (a retry). The rows' `INSERT`s, 13 to a statement for
/// D1's limit of 100 parameters, run only if the guard row is
/// there; the last statement deletes it. Returns whether the guard admitted
/// the add: change counts on D1 include trigger writes (each entrant bumps
/// the raffle's version), but `batch_guards` has no trigger.
///
/// A row whose email key is already in the raffle is left out by
/// `ON CONFLICT`: a join that landed since the caller looked. The caller
/// reads back which rows went in ([`of_batch`]). A taken registration ID or
/// entrant ID is an [`Error::Unique`](crate::Error::Unique), and nothing is
/// added. They saw no join form and accepted nothing, so no consent is
/// recorded for them, and no device holds their entry (no token).
pub async fn insert_added(db: &Db, add: &AddBatch<'_>) -> Result<bool> {
    let key = guard_key(add.raffle_id);
    let test_mode = if add.origin.admits_test_mode() {
        ""
    } else {
        " AND test_mode = 0"
    };
    let mut batch = vec![
        db.query(format!(
            "INSERT INTO batch_guards (key)
             SELECT ?1 WHERE EXISTS (SELECT 1 FROM raffles WHERE id = ?2 AND state IN ({admits})
                                     AND erase_at > ?5{test_mode})
               AND {ROOM}
               AND NOT EXISTS (SELECT 1 FROM entrants WHERE raffle_id = ?2 AND batch_id = ?6)",
            admits = crate::raffles::states(|s| add.origin.admits(s)),
        ))
        .bind(key.as_str())
        .bind(add.raffle_id)
        .bind(add.rows.len())
        .bind(add.max_entrants)
        .bind(add.now)
        .bind(add.batch_id),
    ];
    // ?1 to ?4 are the same for every row; each row binds seven after them.
    for chunk in add.rows.chunks(ADDED_PER_INSERT) {
        let values: Vec<String> = (0..chunk.len())
            .map(|r| {
                let p = 5 + 7 * r;
                // The row number is cast: a statement whose rows all have
                // none would leave Postgres a column of untyped NULLs, which
                // it reads as text.
                format!(
                    "(?{p}, ?2, ?{}, ?{}, ?{}, '{}', ?{}, ?3, ?4, CAST(?{} AS BIGINT), ?{})",
                    p + 1,
                    p + 2,
                    p + 3,
                    Source::Manual.as_str(),
                    p + 4,
                    p + 5,
                    p + 6,
                )
            })
            .collect();
        let insert = db
            .query(format!(
                "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, source,
                                       claim_code, created_at, batch_id, batch_row, list_ref)
                 SELECT * FROM (VALUES {}) AS added
                 WHERE {GUARDED}
                 ON CONFLICT (raffle_id, email_norm) DO NOTHING",
                values.join(", ")
            ))
            .bind(key.as_str())
            .bind(add.raffle_id)
            .bind(add.created_at)
            .bind(add.batch_id);
        batch.push(chunk.iter().fold(insert, |q, a| {
            q.bind(a.id)
                .bind(a.display_name)
                .bind(a.email_raw)
                .bind(a.email_norm)
                .bind(a.claim_code)
                .bind(a.row)
                .bind(a.list_ref)
        }));
    }
    batch.push(release(db, &key));
    let changed = db.batch(batch).await?;
    Ok(changed.first().is_some_and(|&n| n > 0))
}

/// The entrants add `batch_id` put in raffle `raffle_id`, in its order:
/// what an add reads back, and what a retried one answers with.
pub async fn of_batch(db: &Db, raffle_id: &str, batch_id: &str) -> Result<Vec<EntrantRow>> {
    db.query(format!(
        "SELECT * FROM entrants WHERE raffle_id = ? AND batch_id = ? {ORDER}"
    ))
    .bind(raffle_id)
    .bind(batch_id)
    .all()
    .await
}

/// Removes the entrants add `batch_id` put in raffle `raffle_id` (undoing
/// it), only while the entrant list can change. Returns how many: counted in
/// the same transaction as the delete and under the same condition, since
/// D1's change count would include the triggers' writes.
pub async fn delete_batch(db: &Db, raffle_id: &str, batch_id: &str) -> Result<u64> {
    let in_batch = format!("raffle_id = ?1 AND batch_id = ?2 AND {}", editable(1));
    let mut sets = db
        .batch_rows(vec![
            db.query(format!("SELECT COUNT(*) FROM entrants WHERE {in_batch}"))
                .bind(raffle_id)
                .bind(batch_id),
            db.query(format!("DELETE FROM entrants WHERE {in_batch}"))
                .bind(raffle_id)
                .bind(batch_id),
        ])
        .await?
        .into_iter();
    let count = match sets.next().and_then(|rows| rows.into_iter().next()) {
        Some(row) => row.decode::<(i64,)>()?.0,
        None => 0,
    };
    Ok(count.max(0) as u64)
}

/// What a raffle's entrants already hold that an add must not repeat.
#[derive(Debug, Clone, Default)]
pub struct Taken {
    /// How many entrants there are, erased ones included (the cap counts
    /// them, as the add's guard does).
    pub entrants: usize,
    /// Email keys ([`EntrantRow::email_norm`]).
    pub emails: std::collections::HashSet<String>,
    /// Registration IDs.
    pub list_refs: std::collections::HashSet<String>,
    /// Claim codes in use.
    pub claim_codes: std::collections::HashSet<String>,
}

/// How many entrants raffle `raffle_id` has, and the email keys,
/// registration IDs and claim codes they hold, in one read.
pub async fn taken(db: &Db, raffle_id: &str) -> Result<Taken> {
    let rows: Vec<(Option<String>, Option<String>, Option<String>)> = db
        .query("SELECT email_norm, list_ref, claim_code FROM entrants WHERE raffle_id = ?")
        .bind(raffle_id)
        .all()
        .await?;
    let mut taken = Taken {
        entrants: rows.len(),
        ..Taken::default()
    };
    for (email, list_ref, code) in rows {
        taken.emails.extend(email);
        taken.list_refs.extend(list_ref);
        taken.claim_codes.extend(code);
    }
    Ok(taken)
}

/// A fake entrant, for rehearsals.
#[derive(Debug, Clone)]
pub struct Fake {
    pub id: String,
    pub display_name: String,
    pub claim_code: String,
}

/// Adds fake entrants, all or nothing, under the same guard as
/// [`insert_added`]: only while the entrant list can change, the raffle is
/// in test mode, and it has room for all of them under `max_entrants`;
/// multi-row `INSERT`s, 32 to a statement, in one batch.
/// They entered at `created_at`, and accepted nothing (no consent is
/// recorded). Returns `false` if the raffle refused them (nothing is added).
pub async fn insert_fakes(
    db: &Db,
    raffle_id: &str,
    created_at: i64,
    fakes: &[Fake],
    max_entrants: i64,
) -> Result<bool> {
    let key = guard_key(raffle_id);
    let mut batch = vec![
        db.query(format!(
            "INSERT INTO batch_guards (key)
             SELECT ?1 WHERE {} AND EXISTS (SELECT 1 FROM raffles WHERE id = ?2 AND test_mode = 1)
               AND {ROOM}",
            editable(2)
        ))
        .bind(key.as_str())
        .bind(raffle_id)
        .bind(fakes.len())
        .bind(max_entrants),
    ];
    // ?1 to ?3 are the same for every row; each row binds its ID, name and
    // claim code after them.
    for chunk in fakes.chunks(FAKES_PER_INSERT) {
        let values: Vec<String> = (0..chunk.len())
            .map(|r| {
                let p = 4 + 3 * r;
                format!(
                    "(?{p}, ?2, ?{}, '{}', ?{}, ?3)",
                    p + 1,
                    Source::Fake.as_str(),
                    p + 2
                )
            })
            .collect();
        let insert = db
            .query(format!(
                "INSERT INTO entrants (id, raffle_id, display_name, source, claim_code, created_at)
                 SELECT * FROM (VALUES {}) AS fakes
                 WHERE {GUARDED}",
                values.join(", "),
            ))
            .bind(key.as_str())
            .bind(raffle_id)
            .bind(created_at);
        batch.push(chunk.iter().fold(insert, |q, f| {
            q.bind(&f.id).bind(&f.display_name).bind(&f.claim_code)
        }));
    }
    batch.push(release(db, &key));
    let changed = db.batch(batch).await?;
    Ok(changed.first().is_some_and(|&n| n > 0))
}

/// Removes a raffle's fake entrants if it isn't in test mode, while its
/// entrant list can change: what switching test mode off does
/// ([`raffles::update_settings`](crate::raffles::update_settings) runs it in
/// the same batch).
pub(crate) fn delete_stray_fakes<'a>(db: &'a Db, raffle_id: &str) -> crate::Query<'a> {
    db.query(format!(
        "DELETE FROM entrants WHERE raffle_id = ?1 AND source = '{}'
         AND {} AND EXISTS (SELECT 1 FROM raffles WHERE id = ?1 AND test_mode = 0)",
        Source::Fake.as_str(),
        editable(1)
    ))
    .bind(raffle_id)
}

/// Changes to an entrant the organizer makes, applied together.
#[derive(Debug, Clone, Default)]
pub struct Patch<'a> {
    pub display_name: Option<&'a str>,
    pub hidden: Option<bool>,
    /// Delivered at (`Some(Some(at))`), or not delivered (`Some(None)`).
    pub delivered_at: Option<Option<i64>>,
}

/// Applies `patch` to an entrant of `raffle_id` whose data isn't erased, in
/// one statement: all of it or nothing. Returns `false` if there is no such
/// entrant.
pub async fn patch(db: &Db, raffle_id: &str, id: &str, patch: &Patch<'_>) -> Result<bool> {
    // An absent field keeps its value: `?n IS NULL` skips it (`delivered_at`
    // is nullable, so it has its own flag).
    let changed = db
        .query(
            "UPDATE entrants SET
                 display_name = COALESCE(?1, display_name),
                 hidden = COALESCE(?2, hidden),
                 delivered_at = CASE WHEN ?3 = 1 THEN ?4 ELSE delivered_at END
             WHERE raffle_id = ?5 AND id = ?6 AND erased = 0",
        )
        .bind(patch.display_name)
        .bind(patch.hidden)
        .bind(i64::from(patch.delivered_at.is_some()))
        .bind(patch.delivered_at.flatten())
        .bind(raffle_id)
        .bind(id)
        .run()
        .await?;
    Ok(changed > 0)
}

/// Removes an entrant entirely, only while the entrant list can change
/// (afterwards their ID is in the published list: [`erase`] them). Returns
/// `false` if nothing was removed: no such entrant, or the list can't change.
pub async fn delete(db: &Db, raffle_id: &str, id: &str) -> Result<bool> {
    let deleted = db
        .query(format!(
            "DELETE FROM entrants WHERE id = ?1 AND raffle_id = ?2 AND {}",
            editable(2)
        ))
        .bind(id)
        .bind(raffle_id)
        .run()
        .await?;
    Ok(deleted > 0)
}

/// Erases the personal data of one entrant, or of every entrant of the raffle
/// (`None`), and logs each erasure in `erasures` (IDs and time only), so a
/// restored backup can re-apply it. Only when the lifecycle allows
/// ([`RaffleState::personal_data_erasable`]: the raffle is over, or its
/// erasure date has passed at `now`): until then the entry is part of the
/// raffle. One batch: both or neither. Returns whether anyone was erased.
pub async fn erase(db: &Db, raffle_id: &str, entrant_id: Option<&str>, now: i64) -> Result<bool> {
    let erasable = format!(
        "EXISTS (SELECT 1 FROM raffles WHERE id = ?1
                 AND (state IN ({before}) OR (state IN ({past}) AND erase_at <= ?3)))",
        before = crate::raffles::states(|s| s.personal_data_erasable(false)),
        past = crate::raffles::states(|s| s.personal_data_erasable(true)),
    );
    let changed = db
        .batch(vec![
            db.query(format!(
                "INSERT INTO erasures (raffle_id, entrant_id, at)
                 SELECT raffle_id, id, ?3 FROM entrants
                 WHERE raffle_id = ?1 AND erased = 0 AND (?2 IS NULL OR id = ?2) AND {erasable}",
            ))
            .bind(raffle_id)
            .bind(entrant_id)
            .bind(now),
            db.query(format!(
                "UPDATE entrants SET {ERASED}
                 WHERE raffle_id = ?1 AND erased = 0 AND (?2 IS NULL OR id = ?2) AND {erasable}"
            ))
            .bind(raffle_id)
            .bind(entrant_id)
            .bind(now),
        ])
        .await?;
    Ok(changed.get(1).is_some_and(|&n| n > 0))
}

/// Marks every claimed winner's prize delivered at `now`, if it wasn't:
/// their data then goes on the non-winner clock. Returns how many.
pub async fn deliver_all(db: &Db, raffle_id: &str, now: i64) -> Result<u64> {
    let due = format!(
        "raffle_id = ?1 AND erased = 0 AND delivered_at IS NULL
         AND id IN (SELECT entrant_id FROM draws WHERE raffle_id = ?1 AND outcome = '{}')",
        Outcome::Claimed.as_str()
    );
    let (count,): (i64,) = db
        .query(format!("SELECT COUNT(*) FROM entrants WHERE {due}"))
        .bind(raffle_id)
        .one()
        .await?;
    db.query(format!("UPDATE entrants SET delivered_at = ?2 WHERE {due}"))
        .bind(raffle_id)
        .bind(now)
        .run()
        .await?;
    Ok(count.max(0) as u64)
}

/// Re-applies every erasure the log names to an entrant whose personal data
/// is back (a restored backup): the restore tool's write. Returns how many.
pub async fn reapply_erasures(db: &Db) -> Result<u64> {
    let logged = "erased = 0 AND EXISTS (
        SELECT 1 FROM erasures WHERE erasures.raffle_id = entrants.raffle_id AND erasures.entrant_id = entrants.id)";
    let (count,): (i64,) = db
        .query(format!("SELECT COUNT(*) FROM entrants WHERE {logged}"))
        .one()
        .await?;
    db.query(format!("UPDATE entrants SET {ERASED} WHERE {logged}"))
        .run()
        .await?;
    Ok(count.max(0) as u64)
}
