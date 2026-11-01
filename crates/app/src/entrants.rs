//! Entrants: joining, the organizer's edits, and erasing their data.

use std::collections::HashSet;

use rafi_api::{
    AddRequest, AddResult, AddedEntrant, EntrantPatch, RowProblem, SkipReason, SkippedRow,
};
use rafi_lifecycle::{
    self as lifecycle,
    entrant::{self, Email, JoinRefusal, MAX_ADD_ROWS, NameError, Origin, Removal},
};
use rafi_store::{EntrantRow, RaffleRow, audit, entrants, raffles};

use crate::{
    Actor, App, Error, Limit, Refusal, Result, error::Unfinished, ids, owned, pow,
    view::NAME_DELAY_SECS,
};

/// Version of the consent text shown on the join form: the rules checkbox,
/// the erasure dates and the age line under it. Bumped whenever that text
/// changes. What is stored with an entry is `<this>/age<minimum age>`
/// ([`Policy::consent_version`](crate::Policy::consent_version)), so a row
/// says which text and which age its entrant was shown.
pub const CONSENT_VERSION: &str = "2026-10-21.2";

/// The claim codes in use in the raffle, for [`ids::code_besides`].
async fn claim_codes(app: &App, raffle_id: &str) -> Result<HashSet<String>> {
    Ok(entrants::claim_codes(&app.db, raffle_id)
        .await?
        .into_iter()
        .collect())
}

/// The names already in the raffle: generated ones avoid them.
async fn taken_names(app: &App, raffle_id: &str) -> Result<HashSet<String>> {
    Ok(entrants::display_names(&app.db, raffle_id)
        .await?
        .into_iter()
        .collect())
}

/// A generated name nobody in the raffle has yet, for someone who left theirs
/// blank.
async fn generated_name(app: &App, raffle_id: &str) -> Result<String> {
    Ok(ids::generated_name_besides(
        &taken_names(app, raffle_id).await?,
    ))
}

/// Generated names to suggest on the join form: `n` different ones.
pub fn name_ideas(n: usize) -> Vec<String> {
    let mut ideas: Vec<String> = Vec::with_capacity(n);
    while ideas.len() < n {
        let name = ids::generated_name();
        if !ideas.contains(&name) {
            ideas.push(name);
        }
    }
    ideas
}

/// A join that went through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinOutcome {
    /// A new entrant, with their cookie token.
    Joined { token: String },
    /// That email is already in.
    AlreadyIn,
}

/// Why a join didn't go through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JoinError {
    #[error("registration is not open")]
    Closed,
    #[error("not a display name")]
    Name(NameError),
    #[error("not an email address")]
    Email,
    #[error("the rules were not accepted")]
    Rules,
    #[error("the raffle is full")]
    Full,
    /// Over a join rate limit: too many joins for this raffle or this email
    /// address just now.
    #[error("too many joins; try again later")]
    Limited,
    /// The proof-of-work is missing, invalid, expired or already used.
    #[error("no valid proof-of-work")]
    Proof,
    /// The database failed; trying again later may work.
    #[error("the join failed")]
    Failed,
}

/// What the join form posts.
#[derive(Debug, Clone, Copy)]
pub struct JoinRequest<'a> {
    /// The name to show on the screen; blank for a generated one
    /// ([`ids::generated_name`]).
    pub name: &'a str,
    pub email: &'a str,
    /// The raffle rules and the privacy notice were accepted.
    pub rules: bool,
    /// The solved proof-of-work, as the form posts it (base64 JSON).
    pub proof: &'a str,
}

/// Self-service join from the participant page: the rate limits, then the
/// proof-of-work, then the join itself.
pub async fn join(
    app: &App,
    raffle: &RaffleRow,
    req: &JoinRequest<'_>,
) -> std::result::Result<JoinOutcome, JoinError> {
    // An email that doesn't normalize is refused by the join itself; it gets
    // no per-email bucket, rather than one shared by every malformed address.
    // The bucket's key is a hash: the limiter (Cloudflare's binding, on
    // Workers) only needs it stable, and shouldn't see the address.
    let email_key = Email::parse(req.email).map(|e| e.key);
    let limited = !app.limiter.allow(Limit::JoinPerRaffle, &raffle.id).await
        || match &email_key {
            Some(key) => {
                !app.limiter
                    .allow(
                        Limit::JoinPerEmail,
                        &ids::token_hash(&format!("{}:{key}", raffle.id)),
                    )
                    .await
            }
            None => false,
        };
    if limited {
        return Err(JoinError::Limited);
    }
    match pow::redeem(app, req.proof).await {
        Ok(true) => {}
        Ok(false) => return Err(JoinError::Proof),
        Err(e) => {
            tracing::error!(error = %e, "redeeming a proof-of-work");
            return Err(JoinError::Failed);
        }
    }
    enter(app, raffle, req).await
}

/// The join itself, once the abuse checks passed.
async fn enter(
    app: &App,
    raffle: &RaffleRow,
    req: &JoinRequest<'_>,
) -> std::result::Result<JoinOutcome, JoinError> {
    let JoinRequest {
        name, email, rules, ..
    } = *req;
    let now = app.clock.now();
    // Closed first: nothing else matters then.
    let past = raffle.past_erase_date(now);
    if let Err(JoinRefusal::Closed) = entrant::may_join(raffle.state, past, true) {
        return Err(JoinError::Closed);
    }
    let name = if name.trim().is_empty() {
        None
    } else {
        Some(entrant::display_name(name).map_err(JoinError::Name)?)
    };
    let email = Email::parse(email).ok_or(JoinError::Email)?;
    entrant::may_join(raffle.state, past, rules).map_err(|r| match r {
        JoinRefusal::Closed => JoinError::Closed,
        JoinRefusal::Rules => JoinError::Rules,
    })?;
    let name = match name {
        Some(name) => name,
        None => generated_name(app, &raffle.id)
            .await
            .map_err(|_| JoinError::Failed)?,
    };
    let id = ids::id();
    let token = ids::token();
    let claim = ids::code_besides(
        &mut claim_codes(app, &raffle.id)
            .await
            .map_err(|_| JoinError::Failed)?,
    );
    // Only while registration is open, and only below the entrant cap: the
    // hard limit behind the (soft) rate limits.
    let res = entrants::join(
        &app.db,
        &entrants::Join {
            id: &id,
            raffle_id: &raffle.id,
            display_name: &name,
            email_raw: &email.raw,
            email_norm: &email.key,
            claim_code: &claim,
            token_hash: &ids::token_hash(&token),
            consent_version: &app.policy.consent_version(),
        },
        app.policy.max_entrants,
        now,
    )
    .await;
    match res {
        Ok(false) => return not_joined(app, raffle, &email.key, now).await,
        Ok(true) => {}
        Err(e) if e.is_unique_violation() => return Ok(JoinOutcome::AlreadyIn),
        Err(e) => {
            tracing::error!(error = %e, "join failed");
            return Err(JoinError::Failed);
        }
    }
    // No nudge: a join doesn't change what phones see. Screens poll.
    Ok(JoinOutcome::Joined { token })
}

/// Why a join `now` inserted nothing: registration closed, the email is
/// already in, or the raffle is full.
async fn not_joined(
    app: &App,
    raffle: &RaffleRow,
    email_norm: &str,
    now: i64,
) -> std::result::Result<JoinOutcome, JoinError> {
    let open = matches!(
        raffles::get(&app.db, &raffle.id).await,
        Ok(Some(r)) if r.state.joinable() && !r.past_erase_date(now)
    );
    if !open {
        return Err(JoinError::Closed);
    }
    let already = entrants::email_taken(&app.db, &raffle.id, email_norm)
        .await
        .map_err(|_| JoinError::Failed)?;
    if already {
        return Ok(JoinOutcome::AlreadyIn);
    }
    Err(JoinError::Full)
}

/// Counts an add or an undo against the owner's rate limit
/// ([`Limit::AddPerOwner`]). The limiter sees a hash of the owner key.
async fn limit_adds(app: &App, actor: Actor<'_>) -> Result<()> {
    let key = ids::token_hash(&format!("add:{}", actor.owner));
    if app.limiter.allow(Limit::AddPerOwner, &key).await {
        Ok(())
    } else {
        Err(Refusal::Limited.into())
    }
}

/// Adds people the organizer vouches for (the [`Manual`](entrant::Source::Manual)
/// source): one person from the form, or a list. Every add is one batch,
/// all or nothing ([`entrants::insert_added`]):
///
/// - the rows are checked as a whole ([`entrant::added_rows`]), and any
///   invalid row refuses the add with every row's problem;
/// - a row whose email or registration ID is already in the raffle is
///   skipped, not refused: adding an updated list adds only who is new;
/// - the rest must fit under the entrant cap together, or nothing is added;
/// - the raffle must take an add from its origin ([`entrant::may_add`]),
///   checked here and again in the batch's SQL.
///
/// A retry (the same batch ID, whose add went in) answers as the add did and
/// adds nothing. The add is audited (`add`: its batch ID, origin and counts,
/// never who). They saw no join form, so no consent is recorded for them;
/// they still fix the raffle's rules
/// ([`Source::locks_settings`](entrant::Source::locks_settings)). Nothing on
/// a phone changes, so nothing is notified; screens poll.
pub async fn add_entrants(
    app: &App,
    actor: Actor<'_>,
    raffle_id: &str,
    add: AddRequest,
) -> Result<AddResult> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    limit_adds(app, actor).await?;
    let AddRequest {
        batch_id,
        origin,
        rows,
    } = add;
    if !entrant::batch_id(&batch_id) {
        return Err(Refusal::BatchId.into());
    }
    let sized = match origin {
        Origin::One => rows.len() == 1,
        Origin::List => (1..=MAX_ADD_ROWS).contains(&rows.len()),
    };
    if !sized {
        return Err(Refusal::RowCount(origin).into());
    }
    if let Some(earlier) = earlier_add(app, raffle_id, &batch_id, rows.len()).await? {
        return Ok(earlier);
    }
    let now = app.clock.now();
    entrant::may_add(
        origin,
        raffle.state,
        raffle.test_mode,
        raffle.past_erase_date(now),
    )
    .map_err(Refusal::Add)?;
    let checked = entrant::added_rows(
        rows.iter()
            .map(|r| (r.name.as_str(), r.email.as_deref(), r.list_ref.as_deref())),
    )
    .map_err(|problems| Refusal::Rows {
        origin,
        rows: problems
            .into_iter()
            .map(|(row, problem)| RowProblem {
                row: row_index(row),
                problem,
            })
            .collect(),
    })?;

    let mut taken = entrants::taken(&app.db, raffle_id).await?;
    let mut skipped = Vec::new();
    let mut fresh = Vec::new();
    for (row, person) in checked.iter().enumerate() {
        let already = person
            .email
            .as_ref()
            .is_some_and(|e| taken.emails.contains(&e.key))
            || person
                .list_ref
                .as_ref()
                .is_some_and(|r| taken.list_refs.contains(r));
        if already {
            skipped.push(row);
        } else {
            fresh.push((row, person, ids::id()));
        }
    }
    let left = app.policy.max_entrants - i64::try_from(taken.entrants).unwrap_or(i64::MAX);
    if i64::try_from(fresh.len()).unwrap_or(i64::MAX) > left {
        return Err(Refusal::Room {
            adding: fresh.len(),
            left: left.max(0),
        }
        .into());
    }
    let codes: Vec<String> = fresh
        .iter()
        .map(|_| ids::code_besides(&mut taken.claim_codes))
        .collect();
    let added: Vec<entrants::Added<'_>> = fresh
        .iter()
        .zip(&codes)
        .map(|((row, person, id), code)| entrants::Added {
            id,
            display_name: &person.name,
            email_raw: person.email.as_ref().map(|e| e.raw.as_str()),
            email_norm: person.email.as_ref().map(|e| e.key.as_str()),
            claim_code: code,
            row: (origin == Origin::List).then(|| i64::from(row_index(*row))),
            list_ref: person.list_ref.as_deref(),
        })
        .collect();
    if !added.is_empty() {
        let batch = entrants::AddBatch {
            raffle_id,
            batch_id: &batch_id,
            origin,
            created_at: entered_at(origin, now),
            rows: &added,
            max_entrants: app.policy.max_entrants,
            now,
        };
        match entrants::insert_added(&app.db, &batch).await {
            Ok(true) => {}
            Ok(false) => {
                return not_added(
                    app,
                    raffle_id,
                    &batch_id,
                    origin,
                    rows.len(),
                    added.len(),
                    now,
                )
                .await;
            }
            // A registration ID taken since the read above.
            Err(e) if e.is_unique_violation() => return Err(Refusal::Changed.into()),
            Err(e) => return Err(e.into()),
        }
    }
    // Read back: a row whose email a join took meanwhile was left out.
    let stored: HashSet<String> = entrants::of_batch(&app.db, raffle_id, &batch_id)
        .await?
        .into_iter()
        .map(|e| e.id)
        .collect();
    let mut result = AddResult {
        batch_id,
        added: Vec::new(),
        skipped: Vec::new(),
    };
    for (row, _, id) in fresh {
        if stored.contains(&id) {
            result.added.push(AddedEntrant {
                row: row_index(row),
                id,
            });
        } else {
            skipped.push(row);
        }
    }
    skipped.sort_unstable();
    result.skipped = skipped
        .into_iter()
        .map(|row| SkippedRow {
            row: row_index(row),
            reason: SkipReason::AlreadyIn,
        })
        .collect();
    let detail = format!(
        "batch={} origin={} added={} skipped={}",
        result.batch_id,
        origin.as_str(),
        result.added.len(),
        result.skipped.len()
    );
    audit::record(&app.db, raffle_id, actor, "add", &detail, now).await?;
    Ok(result)
}

/// When the people of an add made `now` from `origin` entered
/// (`created_at`): one person now, so their name reaches the screen after
/// the usual delay, as a join's does; a list [`NAME_DELAY_SECS`] earlier, so
/// its names show at once, in the list's order.
fn entered_at(origin: Origin, now: i64) -> i64 {
    match origin {
        Origin::One => now,
        Origin::List => now - NAME_DELAY_SECS,
    }
}

/// When the add entrant `e` came in with was made, if they came in with
/// one: [`entered_at`] undone, a list's rows told from one person's by
/// their row number. What "Undo last add" goes by.
pub(crate) fn added_at(e: &EntrantRow) -> Option<i64> {
    e.batch_id.as_ref()?;
    Some(match e.batch_row {
        Some(_) => e.created_at + NAME_DELAY_SECS,
        None => e.created_at,
    })
}

/// A row's index in an add, as the API numbers it (at most
/// [`MAX_ADD_ROWS`]).
fn row_index(row: usize) -> u32 {
    u32::try_from(row).unwrap_or(u32::MAX)
}

/// What add `batch_id` of `rows` rows did, if it went in before: a retry is
/// answered with it. Its entrants were added, by their row (one person's
/// is row 0); every other row was skipped (already in, then). `None` if no
/// entrant of it is there.
async fn earlier_add(
    app: &App,
    raffle_id: &str,
    batch_id: &str,
    rows: usize,
) -> Result<Option<AddResult>> {
    let stored = entrants::of_batch(&app.db, raffle_id, batch_id).await?;
    if stored.is_empty() {
        return Ok(None);
    }
    Ok(Some(answer(batch_id, &stored, rows)))
}

/// An add's result from its stored entrants.
fn answer(batch_id: &str, stored: &[EntrantRow], rows: usize) -> AddResult {
    let added: Vec<AddedEntrant> = stored
        .iter()
        .map(|e| AddedEntrant {
            row: e.batch_row.and_then(|r| u32::try_from(r).ok()).unwrap_or(0),
            id: e.id.clone(),
        })
        .collect();
    let in_batch: HashSet<u32> = added.iter().map(|a| a.row).collect();
    AddResult {
        batch_id: batch_id.to_owned(),
        skipped: (0..rows)
            .map(row_index)
            .filter(|row| !in_batch.contains(row))
            .map(|row| SkippedRow {
                row,
                reason: SkipReason::AlreadyIn,
            })
            .collect(),
        added,
    }
}

/// Why add `batch_id`'s guard refused it at `now`: the same batch went in
/// meanwhile (a retry that raced the first try: answered as that one), the
/// raffle no longer takes it, or no longer has room for `adding` more.
async fn not_added(
    app: &App,
    raffle_id: &str,
    batch_id: &str,
    origin: Origin,
    rows: usize,
    adding: usize,
    now: i64,
) -> Result<AddResult> {
    if let Some(earlier) = earlier_add(app, raffle_id, batch_id, rows).await? {
        return Ok(earlier);
    }
    let raffle = raffles::get(&app.db, raffle_id)
        .await?
        .ok_or(Error::NotFound)?;
    entrant::may_add(
        origin,
        raffle.state,
        raffle.test_mode,
        raffle.past_erase_date(now),
    )
    .map_err(Refusal::Add)?;
    let count = entrants::taken(&app.db, raffle_id).await?.entrants;
    let left = app.policy.max_entrants - i64::try_from(count).unwrap_or(i64::MAX);
    if i64::try_from(adding).unwrap_or(i64::MAX) > left {
        return Err(Refusal::Room {
            adding,
            left: left.max(0),
        }
        .into());
    }
    Err(Refusal::Changed.into())
}

/// Takes add `batch_id`'s entrants off the list again, while the list can
/// change; returns how many. Undoing an add that has none left (undone
/// already, or each removed by hand) does nothing and returns 0, so a retry
/// is safe. Audited (`undo_add`, with the batch ID and the count) when it
/// removed anyone. Once the list is frozen, refused: the frozen list is the
/// manifest's.
pub async fn undo_add(app: &App, actor: Actor<'_>, raffle_id: &str, batch_id: &str) -> Result<u32> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    limit_adds(app, actor).await?;
    if !entrant::batch_id(batch_id) {
        return Err(Refusal::BatchId.into());
    }
    if !raffle.state.entrants_editable() {
        return Err(Refusal::EntrantsFrozen.into());
    }
    let removed = entrants::delete_batch(&app.db, raffle_id, batch_id).await?;
    if removed == 0 {
        // Frozen since the read above, or nothing to undo.
        let frozen = raffles::get(&app.db, raffle_id)
            .await?
            .is_some_and(|r| !r.state.entrants_editable());
        if frozen {
            return Err(Refusal::EntrantsFrozen.into());
        }
        return Ok(0);
    }
    let now = app.clock.now();
    let detail = format!("batch={batch_id} removed={removed}");
    audit::record(&app.db, raffle_id, actor, "undo_add", &detail, now).await?;
    Ok(u32::try_from(removed).unwrap_or(u32::MAX))
}

/// Adds up to `n` fake entrants for rehearsals (test mode only), with
/// generated names, up to the entrant cap: as many as fit, and the batch
/// itself refuses if a join or an add took the room meanwhile.
pub async fn add_fakes(app: &App, actor: Actor<'_>, raffle_id: &str, n: u32) -> Result<()> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    let n = entrant::fakes(raffle.test_mode, raffle.state, n).map_err(Refusal::Fakes)?;
    // Up to the cap, as the join would be.
    let room = app.policy.max_entrants - entrants::count_active(&app.db, raffle_id).await?;
    let n = u32::try_from(room.max(0)).map_or(n, |room| n.min(room));
    if n == 0 {
        return Err(Refusal::Full.into());
    }
    let mut taken = taken_names(app, raffle_id).await?;
    let mut codes = claim_codes(app, raffle_id).await?;
    let fakes: Vec<entrants::Fake> = (0..n)
        .map(|_| {
            let display_name = ids::generated_name_besides(&taken);
            taken.insert(display_name.clone());
            entrants::Fake {
                id: ids::id(),
                display_name,
                claim_code: ids::code_besides(&mut codes),
            }
        })
        .collect();
    // Their names show on the screen right away.
    let now = app.clock.now();
    let added = entrants::insert_fakes(
        &app.db,
        raffle_id,
        now - NAME_DELAY_SECS,
        &fakes,
        app.policy.max_entrants,
    )
    .await?;
    if !added {
        // Frozen, out of test mode, or full since the read above.
        let current = raffles::get(&app.db, raffle_id)
            .await?
            .ok_or(Error::NotFound)?;
        return Err(match entrant::fakes(current.test_mode, current.state, n) {
            Err(refusal) => Refusal::Fakes(refusal),
            Ok(_) => Refusal::Full,
        }
        .into());
    }
    audit::record(&app.db, raffle_id, actor, "add_fakes", &n.to_string(), now).await?;
    Ok(())
}

/// The organizer's edits to an entrant: all of them, or none (every field is
/// checked before anything is written, and the write is one statement).
pub async fn patch_entrant(
    app: &App,
    actor: Actor<'_>,
    raffle_id: &str,
    entrant_id: &str,
    patch: EntrantPatch,
) -> Result<()> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    let now = app.clock.now();
    let entrant = entrants::get(&app.db, raffle_id, entrant_id)
        .await?
        .ok_or(Error::NotFound)?;
    if entrant.erased {
        return Err(Refusal::Erased.into());
    }
    // The organizer's name for anyone, up to their limit, whoever chose the
    // one before: a joiner's limit keeps trolls off the screen, which the
    // organizer isn't.
    let name = match &patch.display_name {
        Some(name) => Some(entrant::added_name(name).map_err(Refusal::Name)?),
        None => None,
    };
    if patch.delivered.is_some()
        && !lifecycle::raffle::deliverable(
            raffles::drawn(&app.db, &raffle).await?.as_ref(),
            entrant_id,
        )
    {
        return Err(Refusal::NotDeliverable.into());
    }
    let written = entrants::patch(
        &app.db,
        raffle_id,
        entrant_id,
        &entrants::Patch {
            display_name: name.as_deref(),
            hidden: patch.hidden,
            delivered_at: patch.delivered.map(|d| d.then_some(now)),
        },
    )
    .await?;
    if !written {
        // Erased since the read above.
        return Err(Refusal::Erased.into());
    }
    // The action names what changed: delivery moves a winner onto the
    // erasure clock, which the log should say outright.
    let action = match patch.delivered {
        Some(true) => "deliver",
        Some(false) => "undeliver",
        None => "patch_entrant",
    };
    audit::record(&app.db, raffle_id, actor, action, entrant_id, now).await?;
    // A drawn entrant's name is on the phones.
    if patch.display_name.is_some() || patch.hidden.is_some() {
        app.live_updates.notify(raffle_id).await;
    }
    Ok(())
}

/// Removes an entrant before the freeze; once the raffle is over, or its
/// erasure date has passed, erases their personal data (the anonymous ID
/// must stay in the published list). In between, nobody is removed
/// ([`RaffleState::removal`](lifecycle::RaffleState::removal)).
pub async fn remove_entrant(
    app: &App,
    actor: Actor<'_>,
    raffle_id: &str,
    entrant_id: &str,
) -> Result<()> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    let now = app.clock.now();
    let removal = match raffle.state.removal(raffle.past_erase_date(now)) {
        Some(Removal::Delete) if entrants::delete(&app.db, raffle_id, entrant_id).await? => {
            Some(Removal::Delete)
        }
        // Nothing deleted: frozen since the read above (then refused), or no
        // such entrant (nothing to do).
        Some(Removal::Delete) => match raffles::get(&app.db, raffle_id).await? {
            Some(r) if r.state.removal(r.past_erase_date(now)).is_none() => {
                return Err(Refusal::NotRemovable.into());
            }
            _ => None,
        },
        Some(Removal::Erase) => entrants::erase(&app.db, raffle_id, Some(entrant_id), now)
            .await?
            .then_some(Removal::Erase),
        None => return Err(Refusal::NotRemovable.into()),
    };
    match removal {
        Some(Removal::Delete) => {
            audit::record(&app.db, raffle_id, actor, "remove_entrant", entrant_id, now).await?;
        }
        Some(Removal::Erase) => {
            audit::record(&app.db, raffle_id, actor, "erase_entrant", entrant_id, now).await?;
        }
        None => return Ok(()),
    }
    app.live_updates.notify(raffle_id).await;
    Ok(())
}

/// Erases every entrant's personal data of a finished raffle now, and ends
/// its live channel.
pub async fn purge(app: &App, actor: Actor<'_>, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    if raffle.state.apply(lifecycle::Command::Purge).is_none() {
        return Err(Refusal::NotOver(Unfinished::Purge).into());
    }
    let now = app.clock.now();
    entrants::erase(&app.db, raffle_id, None, now).await?;
    raffles::mark_purged(&app.db, raffle_id, now).await?;
    audit::record(&app.db, raffle_id, actor, "purge", "", now).await?;
    // Destroying the channel publishes the final (erased) view first.
    app.live_updates.end(raffle_id).await?;
    Ok(())
}

/// Marks every winner's prize of a finished raffle delivered, so their
/// personal data goes on the non-winner clock instead of being held.
/// Returns how many were.
pub async fn deliver_all(app: &App, actor: Actor<'_>, raffle_id: &str) -> Result<u64> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    if !raffle.state.is_over() {
        return Err(Refusal::NotOver(Unfinished::DeliverAll).into());
    }
    let now = app.clock.now();
    let delivered = entrants::deliver_all(&app.db, raffle_id, now).await?;
    if delivered > 0 {
        audit::record(
            &app.db,
            raffle_id,
            actor,
            "deliver_all",
            &delivered.to_string(),
            now,
        )
        .await?;
    }
    Ok(delivered)
}

/// Ends live updates of a finished raffle: phones stop listening and the
/// raffle's live channel is destroyed. Safe to repeat (the channel is
/// destroyed again).
pub async fn end_live(app: &App, actor: Actor<'_>, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, actor.owner, raffle_id).await?;
    if !raffle.state.is_over() {
        return Err(Refusal::NotOver(Unfinished::EndLive).into());
    }
    let now = app.clock.now();
    raffles::end_live(&app.db, raffle_id, now).await?;
    if raffle.live_ended_at.is_none() {
        audit::record(&app.db, raffle_id, actor, "end_live", "", now).await?;
    }
    app.live_updates.end(raffle_id).await?;
    Ok(())
}
