//! Raffles: their settings and lifecycle.
//!
//! The freeze is three steps, since D1 has no interactive transactions:
//! [`freeze`] closes registration, the caller reads the now-fixed entrant list
//! ([`entrants::active_ids`](crate::entrants::active_ids)), then
//! [`set_manifest`] stores the raffle's manifest (or [`unfreeze`] reverts when nobody is left).
//! Each step is guarded by the state it expects, so retrying is safe.

use rafi_lifecycle::RaffleState;
use rafi_raffle::Raffle;
use serde::Deserialize;

use crate::{Db, Query, Result, now, prizes::PrizeFields};

/// The states `rule` holds for, as an SQL list of their names: SQL that
/// guards on a state asks the raffle's rules ([`RaffleState`]) which.
pub(crate) fn states(rule: impl Fn(RaffleState) -> bool) -> String {
    RaffleState::ALL
        .into_iter()
        .filter(|s| rule(*s))
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A row of `raffles`.
#[derive(Debug, Clone, Deserialize)]
pub struct RaffleRow {
    pub id: String,
    pub owner_id: String,
    pub slug: String,
    pub join_code: String,
    pub title: String,
    pub description: String,
    pub organizer_name: String,
    pub contact_email: String,
    pub rules: String,
    pub event_date: String,
    pub min_age: i64,
    /// Decoding fails on a state rafi doesn't know: never guessed.
    pub state: RaffleState,
    pub test_mode: bool,
    pub manifest: Option<String>,
    pub manifest_hash: Option<String>,
    pub frozen_at: Option<i64>,
    /// The raffle as its engine drew it ([`crate::save_raffle`]): the
    /// manifest and the event log.
    pub log: Option<String>,
    pub log_version: i64,
    pub finished_at: Option<i64>,
    pub purge_nonwinners_at: Option<i64>,
    pub purge_winners_at: Option<i64>,
    pub purged_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Bumped by every write that can change what screens or phones show
    /// (triggers, migration 0002). Live views carry it as `v`.
    pub version: i64,
    /// When live updates ended: the raffle's live channel is gone.
    pub live_ended_at: Option<i64>,
}

impl RaffleRow {
    /// The raffle as drawn, once the engine has opened it.
    pub fn drawn(&self) -> Result<Option<Raffle>> {
        self.log.as_deref().map(crate::parse_raffle).transpose()
    }
}

/// A raffle in its owner's list.
#[derive(Debug, Clone, Deserialize)]
pub struct RaffleSummaryRow {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub state: RaffleState,
    pub test_mode: bool,
    pub created_at: i64,
    /// Every entrant, erased ones included.
    pub entrant_count: i64,
}

/// A new draft raffle.
#[derive(Debug, Clone)]
pub struct NewRaffle<'a> {
    pub id: &'a str,
    pub owner_id: &'a str,
    pub slug: &'a str,
    pub join_code: &'a str,
    pub title: &'a str,
}

/// The settings an organizer edits, already validated.
#[derive(Debug, Clone)]
pub struct Settings<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub organizer_name: &'a str,
    pub contact_email: &'a str,
    pub rules: &'a str,
    pub event_date: &'a str,
    pub min_age: u32,
    pub test_mode: bool,
}

pub async fn get(db: &Db, id: &str) -> Result<Option<RaffleRow>> {
    db.query("SELECT * FROM raffles WHERE id = ?")
        .bind(id)
        .optional()
        .await
}

pub async fn by_slug(db: &Db, slug: &str) -> Result<Option<RaffleRow>> {
    db.query("SELECT * FROM raffles WHERE slug = ?")
        .bind(slug)
        .optional()
        .await
}

pub async fn by_join_code(db: &Db, code: &str) -> Result<Option<RaffleRow>> {
    db.query("SELECT * FROM raffles WHERE join_code = ?")
        .bind(code)
        .optional()
        .await
}

/// An owner's raffles, newest first.
pub async fn list_by_owner(db: &Db, owner_id: &str) -> Result<Vec<RaffleSummaryRow>> {
    db.query(
        "SELECT r.id, r.slug, r.title, r.state, r.test_mode, r.created_at,
                (SELECT COUNT(*) FROM entrants e WHERE e.raffle_id = r.id) AS entrant_count
         FROM raffles r WHERE r.owner_id = ? ORDER BY r.created_at DESC",
    )
    .bind(owner_id)
    .all()
    .await
}

/// Creates a draft raffle. A taken slug or join code is
/// an [`Error::Unique`](crate::Error::Unique).
pub async fn insert(db: &Db, r: &NewRaffle<'_>) -> Result<()> {
    insert_query(db, r).run().await.map(|_| ())
}

fn insert_query<'a>(db: &'a Db, r: &NewRaffle<'_>) -> Query<'a> {
    let now = now();
    db.query(
        "INSERT INTO raffles (id, owner_id, slug, join_code, title, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(r.id)
    .bind(r.owner_id)
    .bind(r.slug)
    .bind(r.join_code)
    .bind(r.title)
    .bind(now)
    .bind(now)
}

pub async fn update_settings(db: &Db, id: &str, s: &Settings<'_>) -> Result<()> {
    settings_query(db, id, s).run().await.map(|_| ())
}

fn settings_query<'a>(db: &'a Db, id: &str, s: &Settings<'_>) -> Query<'a> {
    db.query(
        "UPDATE raffles SET title = ?, description = ?, organizer_name = ?, contact_email = ?, rules = ?,
         event_date = ?, min_age = ?, test_mode = ?, updated_at = ? WHERE id = ?",
    )
    .bind(s.title)
    .bind(s.description)
    .bind(s.organizer_name)
    .bind(s.contact_email)
    .bind(s.rules)
    .bind(s.event_date)
    .bind(s.min_age)
    .bind(s.test_mode)
    .bind(now())
    .bind(id)
}

/// Creates a draft raffle with `settings` and `prizes` (IDs and fields) in
/// one batch: all of it or nothing. A taken slug or join code is an
/// [`Error::Unique`](crate::Error::Unique).
pub async fn insert_with(
    db: &Db,
    r: &NewRaffle<'_>,
    settings: &Settings<'_>,
    prizes: &[(String, PrizeFields<'_>)],
) -> Result<()> {
    let mut queries = vec![insert_query(db, r), settings_query(db, r.id, settings)];
    queries.extend(
        prizes
            .iter()
            .map(|(id, prize)| crate::prizes::insert_query(db, id, r.id, prize)),
    );
    db.batch(queries).await.map(|_| ())
}

/// Deletes the raffle with its prizes and entrants.
pub async fn delete(db: &Db, id: &str) -> Result<()> {
    db.query("DELETE FROM raffles WHERE id = ?")
        .bind(id)
        .run()
        .await
        .map(|_| ())
}

/// Moves the raffle from `from` to `to`. Returns `false` (and changes
/// nothing) if it isn't in `from` any more.
pub async fn transition(db: &Db, id: &str, from: RaffleState, to: RaffleState) -> Result<bool> {
    let changed = db
        .query("UPDATE raffles SET state = ?, updated_at = ? WHERE id = ? AND state = ?")
        .bind(to.as_str())
        .bind(now())
        .bind(id)
        .bind(from.as_str())
        .run()
        .await?;
    Ok(changed > 0)
}

/// The freeze's first step: closes registration of a raffle in `from` (which
/// the lifecycle allows to freeze, [`RaffleState::apply`]). Returns `false` if
/// it isn't in `from` any more.
pub async fn freeze(db: &Db, id: &str, from: RaffleState) -> Result<bool> {
    transition(db, id, from, RaffleState::Frozen).await
}

/// Reverts a freeze that found nobody to draw from: a frozen raffle without a
/// manifest goes back to `to` ([`RaffleState::unfreeze`]: the state it froze
/// from). It undoes the freeze's first step, the store having no transaction
/// to roll back.
pub async fn unfreeze(db: &Db, id: &str, to: RaffleState) -> Result<()> {
    db.query(
        "UPDATE raffles SET state = ?, updated_at = ?
         WHERE id = ? AND state = 'frozen' AND manifest IS NULL",
    )
    .bind(to.as_str())
    .bind(now())
    .bind(id)
    .run()
    .await
    .map(|_| ())
}

/// The freeze's last step: stores the manifest and its hash. Returns `false`
/// if another attempt stored one first (or the raffle isn't frozen any more).
pub async fn set_manifest(db: &Db, id: &str, manifest: &str, manifest_hash: &str) -> Result<bool> {
    let changed = db
        .query(
            "UPDATE raffles SET manifest = ?, manifest_hash = ?, frozen_at = ?
             WHERE id = ? AND state = 'frozen' AND manifest IS NULL",
        )
        .bind(manifest)
        .bind(manifest_hash)
        .bind(now())
        .bind(id)
        .run()
        .await?;
    Ok(changed > 0)
}

/// Marks the raffle purged (its entrants already erased) and ends its live
/// updates.
pub async fn mark_purged(db: &Db, id: &str) -> Result<()> {
    db.query(
        "UPDATE raffles SET state = 'purged', purged_at = ?1, updated_at = ?1,
         live_ended_at = COALESCE(live_ended_at, ?1) WHERE id = ?2",
    )
    .bind(now())
    .bind(id)
    .run()
    .await
    .map(|_| ())
}

/// Records that live updates ended, unless they already had.
pub async fn end_live(db: &Db, id: &str) -> Result<()> {
    let now = now();
    db.query(
        "UPDATE raffles SET live_ended_at = COALESCE(live_ended_at, ?), updated_at = ?
         WHERE id = ?",
    )
    .bind(now)
    .bind(now)
    .bind(id)
    .run()
    .await
    .map(|_| ())
}

/// How many real (not test mode) raffles are under way
/// ([`RaffleState::in_progress`]).
pub async fn live_count(db: &Db) -> Result<i64> {
    let (n,): (i64,) = db
        .query(format!(
            "SELECT COUNT(*) FROM raffles WHERE state IN ({}) AND test_mode = 0",
            states(RaffleState::in_progress)
        ))
        .one()
        .await?;
    Ok(n)
}
