//! A raffle's entrants, and erasing their personal data.

use rafi_lifecycle::{RaffleState, entrant::Source};
use rafi_raffle::Outcome;
use serde::Deserialize;

use crate::{Db, Result, now};

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
    /// Their personal data was erased ([`ERASED`]): the row stays, anonymous.
    pub erased: bool,
    pub created_at: i64,
}

/// The order entrants are listed in: by when they entered.
pub(crate) const ORDER: &str = "ORDER BY created_at, id";

/// The `SET` clause that erases an entrant's personal data: their name, email,
/// cookie token and claim code. The anonymous ID and the draw log stay.
pub(crate) const ERASED: &str = "display_name = '#' || substr(id, 1, 6), email_raw = NULL, \
     email_norm = NULL, token_hash = NULL, claim_code = NULL, erased = 1";

/// Rows per `INSERT` in [`insert_fakes`]: 2 shared parameters plus 3 per row
/// stays within D1's 100 bound parameters per statement.
const FAKES_PER_INSERT: usize = 32;

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
/// that already entered is an [`Error::Unique`](crate::Error::Unique).
pub async fn join(db: &Db, j: &Join<'_>, max_entrants: i64) -> Result<bool> {
    let now = now();
    let inserted = db
        .query(format!(
            "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, source, claim_code, token_hash,
                                   consent_version, consent_rules_at, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, '{source}', ?6, ?7, ?8, ?9, ?10
             WHERE EXISTS (SELECT 1 FROM raffles WHERE id = ?2 AND state IN ({joinable}))
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

/// An entrant the organizer adds by hand.
#[derive(Debug, Clone)]
pub struct Manual<'a> {
    pub id: &'a str,
    pub raffle_id: &'a str,
    pub display_name: &'a str,
    pub email_raw: Option<&'a str>,
    pub email_norm: Option<&'a str>,
    pub claim_code: &'a str,
    pub consent_version: &'a str,
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

/// Adds an entrant by hand, only while the entrant list can change. Returns
/// `false` if it can't. An email that already entered is an
/// [`Error::Unique`](crate::Error::Unique).
pub async fn insert_manual(db: &Db, m: &Manual<'_>) -> Result<bool> {
    let now = now();
    let inserted = db
        .query(format!(
            "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, source, claim_code,
                                   consent_version, consent_rules_at, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, '{}', ?6, ?7, ?8, ?9
             WHERE {}",
            Source::Manual.as_str(),
            editable(2)
        ))
        .bind(m.id)
        .bind(m.raffle_id)
        .bind(m.display_name)
        .bind(m.email_raw)
        .bind(m.email_norm)
        .bind(m.claim_code)
        .bind(m.consent_version)
        .bind(now)
        .bind(now)
        .run()
        .await?;
    Ok(inserted > 0)
}

/// A fake entrant, for rehearsals.
#[derive(Debug, Clone)]
pub struct Fake {
    pub id: String,
    pub display_name: String,
    pub claim_code: String,
}

/// Adds fake entrants, all or nothing: multi-row `INSERT`s in one batch,
/// only while the entrant list can change and the raffle is in test mode.
/// They entered at `created_at`, and accepted nothing (no consent is
/// recorded). Returns `false` if the list can't change or the raffle isn't
/// in test mode (nothing is added).
pub async fn insert_fakes(
    db: &Db,
    raffle_id: &str,
    created_at: i64,
    fakes: &[Fake],
) -> Result<bool> {
    // ?1 and ?2 are the same for every row; each row binds its ID, name and
    // claim code after them.
    let inserts = fakes
        .chunks(FAKES_PER_INSERT)
        .map(|chunk| {
            let values: Vec<String> = (0..chunk.len())
                .map(|r| {
                    let p = 3 + 3 * r;
                    format!(
                        "(?{p}, ?1, ?{}, '{}', ?{}, ?2)",
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
                     WHERE {} AND EXISTS (SELECT 1 FROM raffles WHERE id = ?1 AND test_mode = 1)",
                    values.join(", "),
                    editable(1)
                ))
                .bind(raffle_id)
                .bind(created_at);
            chunk.iter().fold(insert, |q, f| {
                q.bind(&f.id).bind(&f.display_name).bind(&f.claim_code)
            })
        })
        .collect();
    let inserted = db.batch(inserts).await?;
    Ok(fakes.is_empty() || inserted.iter().any(|&n| n > 0))
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
