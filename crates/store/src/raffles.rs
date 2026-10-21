//! Raffles: their settings and lifecycle.
//!
//! The freeze is three steps, since D1 has no interactive transactions:
//! [`freeze`] closes registration, the caller reads the now-fixed entrant list
//! ([`entrants::active_ids`](crate::entrants::active_ids)), then
//! [`set_manifest`] stores the raffle's manifest (or [`unfreeze`] reverts when nobody is left).
//! Each step is guarded by the state it expects, so retrying is safe.

use rafi_lifecycle::{RaffleState, entrant::Source};
use rafi_raffle::{Event, Manifest, Raffle};
use serde::Deserialize;

use crate::{Db, Error, Query, Result, Row, now, prizes::PrizeFields};

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
    /// Where privacy requests go; blank means `contact_email`.
    pub privacy_email: String,
    /// The organizer's own privacy text and notice, shown after the
    /// generated notice.
    pub privacy_text: String,
    pub privacy_url: String,
    pub rules: String,
    /// The event's calendar day, `YYYY-MM-DD`.
    pub event_date: String,
    /// The instant the event day ends where the organizer is.
    pub event_ends_at: i64,
    /// Days after the event date before non-winners are erased.
    pub retention_days: i64,
    /// When non-winners are erased: `event_ends_at` plus the period. Written
    /// by the settings write only, and fixed once an entrant has accepted the
    /// rules.
    pub erase_at: i64,
    /// Decoding fails on a state rafi doesn't know: never guessed.
    pub state: RaffleState,
    pub test_mode: bool,
    pub manifest: Option<String>,
    pub manifest_hash: Option<String>,
    pub frozen_at: Option<i64>,
    /// When the engine first wrote the raffle ([`crate::save_raffle`]): it is
    /// opened, and [`drawn`] reads it. Opening records no event.
    pub opened_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub purged_at: Option<i64>,
    /// When the owner removed their data: the organizer is gone for good.
    pub organizer_left_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Bumped by every write that can change what screens or phones show
    /// (triggers in the schema). Live views carry it as `v`.
    pub version: i64,
    /// When live updates ended: the raffle's live channel is gone.
    pub live_ended_at: Option<i64>,
}

impl RaffleRow {
    /// Whether the engine has opened the raffle.
    pub fn opened(&self) -> bool {
        self.opened_at.is_some()
    }

    /// Whether the erasure date has passed at `now`.
    pub fn past_erase_date(&self, now: i64) -> bool {
        self.erase_at <= now
    }

    /// The organizer's privacy contact: their privacy email, or else their
    /// contact email.
    pub fn privacy_contact(&self) -> &str {
        if self.privacy_email.is_empty() {
            &self.contact_email
        } else {
            &self.privacy_email
        }
    }
}

/// The raffle as drawn, once the engine has opened it: rebuilt from the row's
/// manifest and its events. The manifest never changes once set and the
/// events only grow, so they make a valid raffle read apart from the row.
pub async fn drawn(db: &Db, row: &RaffleRow) -> Result<Option<Raffle>> {
    if !row.opened() {
        return Ok(None);
    }
    let events = events_query(db, &row.id).rows().await?;
    rebuild(row.opened_at, row.manifest.as_deref(), events)
}

/// Reads raffle `raffle_id`'s events, oldest first: rows for [`rebuild`].
pub(crate) fn events_query<'a>(db: &'a Db, raffle_id: &str) -> Query<'a> {
    db.query("SELECT seq, body FROM raffle_events WHERE raffle_id = ? ORDER BY seq")
        .bind(raffle_id)
}

/// Rebuilds a raffle from its row (when it was opened, its manifest) and its
/// events ([`events_query`]): `None` until it is opened. A raffle that doesn't
/// load (opened without a manifest, events before it opened, an event that
/// doesn't parse, a log that doesn't replay) is an error, never "not opened".
pub(crate) fn rebuild(
    opened_at: Option<i64>,
    manifest: Option<&str>,
    events: Vec<Row>,
) -> Result<Option<Raffle>> {
    let fail = |what: String| Error::Decode(format!("the stored raffle: {what}"));
    if opened_at.is_none() {
        return if events.is_empty() {
            Ok(None)
        } else {
            Err(fail("events, but it was never opened".into()))
        };
    }
    let manifest: Manifest = serde_json::from_str(
        manifest.ok_or_else(|| fail("opened, but it has no manifest".into()))?,
    )
    .map_err(|e| fail(format!("its manifest: {e}")))?;
    let events = events
        .into_iter()
        .map(|row| {
            let (seq, body): (i64, String) = row.decode()?;
            serde_json::from_str::<Event>(&body).map_err(|e| fail(format!("event {seq}: {e}")))
        })
        .collect::<Result<Vec<_>>>()?;
    Raffle::restore(&manifest, &events)
        .map(Some)
        .map_err(|e| fail(e.to_string()))
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
    pub privacy_email: &'a str,
    pub privacy_text: &'a str,
    pub privacy_url: &'a str,
    pub rules: &'a str,
    /// The event's calendar day, `YYYY-MM-DD`.
    pub event_date: &'a str,
    /// The instant the event day ends where the organizer is.
    pub event_ends_at: i64,
    /// Days after the event date before non-winners are erased.
    pub retention_days: i64,
    /// When non-winners are erased: the app computes it from the two above
    /// ([`rafi_lifecycle::settings::erase_at`]); the store writes it with
    /// them and nowhere else.
    pub erase_at: i64,
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

/// An owner's raffles, newest first, whole.
pub async fn rows_by_owner(db: &Db, owner_id: &str) -> Result<Vec<RaffleRow>> {
    db.query("SELECT * FROM raffles WHERE owner_id = ? ORDER BY created_at DESC")
        .bind(owner_id)
        .all()
        .await
}

/// Creates a draft raffle with `settings`. A taken slug or join code is an
/// [`Error::Unique`].
pub async fn insert(db: &Db, r: &NewRaffle<'_>, settings: &Settings<'_>) -> Result<()> {
    insert_query(db, r, settings).run().await.map(|_| ())
}

fn insert_query<'a>(db: &'a Db, r: &NewRaffle<'_>, s: &Settings<'_>) -> Query<'a> {
    let now = now();
    db.query(
        "INSERT INTO raffles (id, owner_id, slug, join_code, title, description, organizer_name, contact_email,
                              privacy_email, privacy_text, privacy_url, rules, event_date, event_ends_at,
                              retention_days, erase_at, test_mode, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(r.id)
    .bind(r.owner_id)
    .bind(r.slug)
    .bind(r.join_code)
    .bind(r.title)
    .bind(s.description)
    .bind(s.organizer_name)
    .bind(s.contact_email)
    .bind(s.privacy_email)
    .bind(s.privacy_text)
    .bind(s.privacy_url)
    .bind(s.rules)
    .bind(s.event_date)
    .bind(s.event_ends_at)
    .bind(s.retention_days)
    .bind(s.erase_at)
    .bind(s.test_mode)
    .bind(now)
    .bind(now)
}

/// What [`update_settings`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsWrite {
    /// The settings were written: `false` if they changed a fixed setting
    /// (the rules, the event date, the retention period, the privacy text or
    /// link) after someone accepted them, or test mode once the list is
    /// frozen.
    pub written: bool,
    /// Whether fake entrants went with test mode. (Change counts are only
    /// zero or not on every backend: D1's include the triggers' writes.)
    pub fakes_removed: bool,
}

/// Writes the settings and, in the same batch, removes the fake entrants of a
/// raffle that is now out of test mode: there are never fakes outside a
/// rehearsal.
pub async fn update_settings(db: &Db, id: &str, s: &Settings<'_>) -> Result<SettingsWrite> {
    let counts = db
        .batch(vec![
            settings_query(db, id, s),
            crate::entrants::delete_stray_fakes(db, id),
        ])
        .await?;
    Ok(SettingsWrite {
        written: counts.first().is_some_and(|&n| n > 0),
        fakes_removed: counts.get(1).is_some_and(|&n| n > 0),
    })
}

/// Writes the settings, unless they change a fixed setting after someone
/// accepted the rules ([`rafi_lifecycle::settings::rules_editable`]: the
/// rules, the event date and with it the erasure date, the retention period,
/// the privacy text and link) or test mode once the entrant list is frozen
/// ([`RaffleState::test_mode_editable`]): the checks and the write are one
/// statement, so no join or freeze slips in between.
fn settings_query<'a>(db: &'a Db, id: &str, s: &Settings<'_>) -> Query<'a> {
    let accepted = Source::ALL
        .into_iter()
        .filter(|s| s.accepted_rules())
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    db.query(format!(
        "UPDATE raffles SET title = ?1, description = ?2, organizer_name = ?3, contact_email = ?4, rules = ?5,
         event_date = ?6, test_mode = ?7, updated_at = ?8, privacy_email = ?10, privacy_text = ?11,
         privacy_url = ?12, event_ends_at = ?13, retention_days = ?14, erase_at = ?15
         WHERE id = ?9
           AND ((rules = ?5 AND event_date = ?6 AND privacy_text = ?11 AND privacy_url = ?12
                 AND event_ends_at = ?13 AND retention_days = ?14)
                OR NOT EXISTS (SELECT 1 FROM entrants WHERE raffle_id = ?9 AND source IN ({accepted})))
           AND (test_mode = ?7 OR state IN ({test_mode_editable}))",
        test_mode_editable = states(RaffleState::test_mode_editable),
    ))
    .bind(s.title)
    .bind(s.description)
    .bind(s.organizer_name)
    .bind(s.contact_email)
    .bind(s.rules)
    .bind(s.event_date)
    .bind(s.test_mode)
    .bind(now())
    .bind(id)
    .bind(s.privacy_email)
    .bind(s.privacy_text)
    .bind(s.privacy_url)
    .bind(s.event_ends_at)
    .bind(s.retention_days)
    .bind(s.erase_at)
}

/// Creates a draft raffle with `settings` and `prizes` (IDs and fields) in
/// one batch: all of it or nothing. A taken slug or join code is an
/// [`Error::Unique`].
pub async fn insert_with(
    db: &Db,
    r: &NewRaffle<'_>,
    settings: &Settings<'_>,
    prizes: &[(String, PrizeFields<'_>)],
) -> Result<()> {
    let mut queries = vec![insert_query(db, r, settings)];
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
    db.query(format!(
        "UPDATE raffles SET state = '{}', purged_at = ?1, updated_at = ?1,
         live_ended_at = COALESCE(live_ended_at, ?1) WHERE id = ?2",
        RaffleState::Purged.as_str()
    ))
    .bind(now())
    .bind(id)
    .run()
    .await
    .map(|_| ())
}

/// What [`owner_leaves`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Left {
    /// The raffles that were never frozen: deleted outright.
    pub deleted: Vec<String>,
    /// The raffles that were over: their entrants erased, the organizer
    /// removed, purged.
    pub purged: Vec<String>,
}

/// The owner's data goes: every raffle of theirs that was never frozen
/// ([`RaffleState::expires`]) is deleted, every one that is over
/// ([`RaffleState::is_over`]) loses its entrants' personal data (logged in
/// `erasures`, audited as a purge) and the organizer's name and contacts,
/// and is purged with its live updates ended; their defaults go. One batch,
/// every write guarded by the state it expects, so a raffle that was frozen
/// meanwhile is left alone: the caller refuses when any is being drawn, and
/// may check [`Left`] against what it saw. The anonymous raffles stay
/// verifiable. Returns the raffles touched, as read before the batch.
pub async fn owner_leaves(db: &Db, owner: &str, now: i64) -> Result<Left> {
    let rows = rows_by_owner(db, owner).await?;
    let (expiring, over) = (states(RaffleState::expires), states(RaffleState::is_over));
    let over_raffle = format!(
        "EXISTS (SELECT 1 FROM raffles WHERE id = entrants.raffle_id AND owner_id = ?1 AND state IN ({over}))"
    );
    db.batch(vec![
        db.query(format!(
            "INSERT INTO audit (raffle_id, at, actor, action, detail)
             SELECT id, ?2, ?1, 'delete', 'owner left' FROM raffles WHERE owner_id = ?1 AND state IN ({expiring})"
        ))
        .bind(owner)
        .bind(now),
        db.query(format!(
            "DELETE FROM raffles WHERE owner_id = ?1 AND state IN ({expiring})"
        ))
        .bind(owner),
        db.query(format!(
            "INSERT INTO audit (raffle_id, at, actor, action, detail)
             SELECT id, ?2, ?1, 'purge', 'owner left' FROM raffles WHERE owner_id = ?1 AND state IN ({over})"
        ))
        .bind(owner)
        .bind(now),
        db.query(format!(
            "INSERT INTO erasures (raffle_id, entrant_id, at)
             SELECT raffle_id, id, ?2 FROM entrants WHERE erased = 0 AND {over_raffle}"
        ))
        .bind(owner)
        .bind(now),
        db.query(format!(
            "UPDATE entrants SET {} WHERE erased = 0 AND {over_raffle}",
            crate::entrants::ERASED
        ))
        .bind(owner),
        db.query(format!(
            "UPDATE raffles SET organizer_name = '', contact_email = '', privacy_email = '',
                    privacy_text = '', privacy_url = '', state = '{purged}',
                    purged_at = COALESCE(purged_at, ?2), organizer_left_at = ?2,
                    live_ended_at = COALESCE(live_ended_at, ?2), updated_at = ?2
             WHERE owner_id = ?1 AND state IN ({over})",
            purged = RaffleState::Purged.as_str(),
        ))
        .bind(owner)
        .bind(now),
        db.query("DELETE FROM owner_defaults WHERE owner_id = ?1").bind(owner),
    ])
    .await?;
    let mut left = Left::default();
    for r in rows {
        if r.state.expires() {
            left.deleted.push(r.id);
        } else if r.state.is_over() {
            left.purged.push(r.id);
        }
    }
    Ok(left)
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
