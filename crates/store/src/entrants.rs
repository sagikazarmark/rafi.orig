//! A raffle's entrants, and erasing their personal data.

use rafi_raffle::RaffleState;
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
    pub source: String,
    pub hidden: bool,
    pub claim_code: String,
    pub token_hash: Option<String>,
    pub consent_version: Option<String>,
    pub consent_rules_at: Option<i64>,
    pub consent_marketing: bool,
    pub delivered_at: Option<i64>,
    pub purged: bool,
    pub created_at: i64,
}

/// The order entrants are listed in: by when they entered.
pub(crate) const ORDER: &str = "ORDER BY created_at, id";

/// The `SET` clause that erases an entrant's personal data. The anonymous
/// ID, the claim code's row and the draw log stay.
pub(crate) const ERASED: &str = "display_name = '#' || substr(id, 1, 6), email_raw = NULL, \
     email_norm = NULL, token_hash = NULL, consent_marketing = 0, purged = 1";

/// Rows per `INSERT` in [`insert_fakes`]: 4 shared parameters plus 3 per row
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
        .query("SELECT COUNT(*) FROM entrants WHERE raffle_id = ? AND purged = 0")
        .bind(raffle_id)
        .one()
        .await?;
    Ok(n)
}

/// The IDs of the entrants that haven't been erased: the list a freeze commits.
pub async fn active_ids(db: &Db, raffle_id: &str) -> Result<Vec<String>> {
    let ids: Vec<(String,)> = db
        .query("SELECT id FROM entrants WHERE raffle_id = ? AND purged = 0")
        .bind(raffle_id)
        .all()
        .await?;
    Ok(ids.into_iter().map(|(id,)| id).collect())
}

/// Every claim code in use in the raffle.
pub async fn claim_codes(db: &Db, raffle_id: &str) -> Result<Vec<String>> {
    let codes: Vec<(String,)> = db
        .query("SELECT claim_code FROM entrants WHERE raffle_id = ?")
        .bind(raffle_id)
        .all()
        .await?;
    Ok(codes.into_iter().map(|(c,)| c).collect())
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
    pub consent_marketing: bool,
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
                                   consent_version, consent_rules_at, consent_marketing, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, 'self', ?6, ?7, ?8, ?9, ?10, ?11
             WHERE EXISTS (SELECT 1 FROM raffles WHERE id = ?2 AND state IN ({joinable}))
               AND (SELECT COUNT(*) FROM entrants WHERE raffle_id = ?2) < ?12",
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
        .bind(j.consent_marketing)
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

/// Adds an entrant by hand. An email that already entered is an
/// [`Error::Unique`](crate::Error::Unique).
pub async fn insert_manual(db: &Db, m: &Manual<'_>) -> Result<()> {
    let now = now();
    db.query(
        "INSERT INTO entrants (id, raffle_id, display_name, email_raw, email_norm, source, claim_code,
                               consent_version, consent_rules_at, created_at)
         VALUES (?, ?, ?, ?, ?, 'manual', ?, ?, ?, ?)",
    )
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
    .await
    .map(|_| ())
}

/// A fake entrant, for rehearsals.
#[derive(Debug, Clone)]
pub struct Fake {
    pub id: String,
    pub display_name: String,
    pub claim_code: String,
}

/// Adds fake entrants, all or nothing: multi-row `INSERT`s in one batch.
/// They entered at `created_at`.
pub async fn insert_fakes(
    db: &Db,
    raffle_id: &str,
    consent_version: &str,
    created_at: i64,
    fakes: &[Fake],
) -> Result<()> {
    let now = now();
    // ?1–?4 are the same for every row; each row binds its ID, name and claim
    // code after them.
    let inserts = fakes
        .chunks(FAKES_PER_INSERT)
        .map(|chunk| {
            let values: Vec<String> = (0..chunk.len())
                .map(|r| {
                    let p = 5 + 3 * r;
                    format!("(?{p}, ?1, ?{}, 'fake', ?{}, ?2, ?3, ?4)", p + 1, p + 2)
                })
                .collect();
            let insert = db
                .query(format!(
                    "INSERT INTO entrants (id, raffle_id, display_name, source, claim_code, consent_version,
                                           consent_rules_at, created_at)
                     VALUES {}",
                    values.join(", ")
                ))
                .bind(raffle_id)
                .bind(consent_version)
                .bind(now)
                .bind(created_at);
            chunk.iter().fold(insert, |q, f| {
                q.bind(&f.id).bind(&f.display_name).bind(&f.claim_code)
            })
        })
        .collect();
    db.batch(inserts).await.map(|_| ())
}

pub async fn set_display_name(db: &Db, id: &str, display_name: &str) -> Result<()> {
    db.query("UPDATE entrants SET display_name = ? WHERE id = ?")
        .bind(display_name)
        .bind(id)
        .run()
        .await
        .map(|_| ())
}

pub async fn set_hidden(db: &Db, id: &str, hidden: bool) -> Result<()> {
    db.query("UPDATE entrants SET hidden = ? WHERE id = ?")
        .bind(hidden)
        .bind(id)
        .run()
        .await
        .map(|_| ())
}

/// Marks the entrant's prize delivered (at `at`), or not delivered (`None`).
pub async fn set_delivered(db: &Db, id: &str, at: Option<i64>) -> Result<()> {
    db.query("UPDATE entrants SET delivered_at = ? WHERE id = ?")
        .bind(at)
        .bind(id)
        .run()
        .await
        .map(|_| ())
}

/// Removes an entrant entirely (before the freeze).
pub async fn delete(db: &Db, raffle_id: &str, id: &str) -> Result<()> {
    db.query("DELETE FROM entrants WHERE id = ? AND raffle_id = ?")
        .bind(id)
        .bind(raffle_id)
        .run()
        .await
        .map(|_| ())
}

/// Erases the personal data of one entrant, or of every entrant of the raffle
/// (`None`), and logs each erasure in `erasures` (IDs and time only), so a
/// restored backup can re-apply it. One batch: both or neither. Returns
/// whether anyone was erased.
pub async fn erase(db: &Db, raffle_id: &str, entrant_id: Option<&str>) -> Result<bool> {
    let changed = db
        .batch(vec![
            db.query(
                "INSERT INTO erasures (raffle_id, entrant_id, at)
                 SELECT raffle_id, id, ?3 FROM entrants
                 WHERE raffle_id = ?1 AND purged = 0 AND (?2 IS NULL OR id = ?2)",
            )
            .bind(raffle_id)
            .bind(entrant_id)
            .bind(now()),
            db.query(format!(
                "UPDATE entrants SET {ERASED}
                 WHERE raffle_id = ?1 AND purged = 0 AND (?2 IS NULL OR id = ?2)"
            ))
            .bind(raffle_id)
            .bind(entrant_id),
        ])
        .await?;
    Ok(changed.get(1).is_some_and(|&n| n > 0))
}
