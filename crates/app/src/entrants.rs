//! Entrants: joining, the organizer's edits, and erasing their data.

use std::collections::HashSet;

use rafi_api::EntrantPatch;
use rafi_lifecycle::{
    self as lifecycle,
    entrant::{self, Email, JoinRefusal, NameError, Removal},
};
use rafi_store::{RaffleRow, audit, entrants, raffles};

use crate::{App, Error, Limit, Refusal, Result, altcha, ids, owned, view::NAME_DELAY_SECS};

/// Version of the consent text shown on the join form.
pub const CONSENT_VERSION: &str = "2026-10-20";

async fn unique_claim_code(app: &App, raffle_id: &str) -> Result<String> {
    let taken: HashSet<String> = entrants::claim_codes(&app.db, raffle_id)
        .await?
        .into_iter()
        .collect();
    for _ in 0..50 {
        let code = ids::code(4);
        if !taken.contains(&code) {
            return Ok(code);
        }
    }
    Ok(ids::code(6))
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
    /// Consent to emails about future meetups.
    pub marketing: bool,
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
    let email_key = Email::parse(req.email).map(|e| e.key);
    let limited = !app.limiter.allow(Limit::JoinPerRaffle, &raffle.id).await
        || match &email_key {
            Some(key) => {
                !app.limiter
                    .allow(Limit::JoinPerEmail, &format!("{}:{key}", raffle.id))
                    .await
            }
            None => false,
        };
    if limited {
        return Err(JoinError::Limited);
    }
    match altcha::redeem(app, req.proof).await {
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
        name,
        email,
        rules,
        marketing,
        ..
    } = *req;
    // Closed first: nothing else matters then.
    if let Err(JoinRefusal::Closed) = entrant::may_join(raffle.state, true) {
        return Err(JoinError::Closed);
    }
    let name = if name.trim().is_empty() {
        None
    } else {
        Some(entrant::display_name(name).map_err(JoinError::Name)?)
    };
    let email = Email::parse(email).ok_or(JoinError::Email)?;
    entrant::may_join(raffle.state, rules).map_err(|r| match r {
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
    let claim = unique_claim_code(app, &raffle.id)
        .await
        .map_err(|_| JoinError::Failed)?;
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
            consent_version: CONSENT_VERSION,
            consent_marketing: marketing,
        },
        app.policy.max_entrants,
    )
    .await;
    match res {
        Ok(false) => return not_joined(app, raffle, &email.key).await,
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

/// Why a join inserted nothing: registration closed, the email is already
/// in, or the raffle is full.
async fn not_joined(
    app: &App,
    raffle: &RaffleRow,
    email_norm: &str,
) -> std::result::Result<JoinOutcome, JoinError> {
    let open = matches!(
        raffles::get(&app.db, &raffle.id).await,
        Ok(Some(r)) if r.state.joinable()
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

/// Adds someone without a phone; returns their entrant ID.
pub async fn add_manual(
    app: &App,
    owner: &str,
    raffle_id: &str,
    name: &str,
    email: Option<String>,
) -> Result<String> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state.entrants_editable() {
        return Err(Refusal::EntrantsFrozen.into());
    }
    let name = entrant::display_name(name).map_err(Refusal::Name)?;
    let email = match email.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        Some(e) => Some(Email::parse(e).ok_or(Refusal::Email)?),
        None => None,
    };
    let id = ids::id();
    let claim = unique_claim_code(app, raffle_id).await?;
    let res = entrants::insert_manual(
        &app.db,
        &entrants::Manual {
            id: &id,
            raffle_id,
            display_name: &name,
            email_raw: email.as_ref().map(|e| e.raw.as_str()),
            email_norm: email.as_ref().map(|e| e.key.as_str()),
            claim_code: &claim,
            consent_version: CONSENT_VERSION,
        },
    )
    .await;
    match res {
        Ok(true) => {}
        // The list froze since the read above.
        Ok(false) => return Err(Refusal::EntrantsFrozen.into()),
        Err(e) if e.is_unique_violation() => return Err(Refusal::EmailTaken.into()),
        Err(e) => return Err(e.into()),
    }
    audit::record(&app.db, raffle_id, owner, "add_manual", &id).await?;
    Ok(id)
}

/// Adds up to `n` fake entrants for rehearsals (test mode only), with
/// generated names.
pub async fn add_fakes(app: &App, owner: &str, raffle_id: &str, n: u32) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    let n = entrant::fakes(raffle.test_mode, raffle.state, n).map_err(Refusal::Fakes)?;
    let mut taken = taken_names(app, raffle_id).await?;
    let fakes: Vec<entrants::Fake> = (0..n)
        .map(|_| {
            let display_name = ids::generated_name_besides(&taken);
            taken.insert(display_name.clone());
            entrants::Fake {
                id: ids::id(),
                display_name,
                claim_code: ids::code(4),
            }
        })
        .collect();
    // Their names show on the screen right away.
    let added =
        entrants::insert_fakes(&app.db, raffle_id, ids::now() - NAME_DELAY_SECS, &fakes).await?;
    if !added {
        // Frozen, or out of test mode, since the read above.
        let now = raffles::get(&app.db, raffle_id)
            .await?
            .ok_or(Error::NotFound)?;
        let refusal = entrant::fakes(now.test_mode, now.state, n)
            .err()
            .unwrap_or(entrant::FakesRefusal::Frozen);
        return Err(Refusal::Fakes(refusal).into());
    }
    audit::record(&app.db, raffle_id, owner, "add_fakes", &n.to_string()).await?;
    Ok(())
}

/// The organizer's edits to an entrant: all of them, or none (every field is
/// checked before anything is written, and the write is one statement).
pub async fn patch_entrant(
    app: &App,
    owner: &str,
    raffle_id: &str,
    entrant_id: &str,
    patch: EntrantPatch,
) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    let entrant = entrants::get(&app.db, raffle_id, entrant_id)
        .await?
        .ok_or(Error::NotFound)?;
    if entrant.erased {
        return Err(Refusal::Erased.into());
    }
    let name = match &patch.display_name {
        Some(name) => Some(entrant::display_name(name).map_err(Refusal::Name)?),
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
            delivered_at: patch.delivered.map(|d| d.then(ids::now)),
        },
    )
    .await?;
    if !written {
        // Erased since the read above.
        return Err(Refusal::Erased.into());
    }
    audit::record(&app.db, raffle_id, owner, "patch_entrant", entrant_id).await?;
    // A drawn entrant's name is on the phones.
    if patch.display_name.is_some() || patch.hidden.is_some() {
        app.live_updates.notify(raffle_id).await;
    }
    Ok(())
}

/// Removes an entrant before the freeze; once the raffle is over, erases their
/// personal data (the anonymous ID must stay in the published list). In
/// between, nobody is removed ([`RaffleState::removal`](lifecycle::RaffleState::removal)).
pub async fn remove_entrant(
    app: &App,
    owner: &str,
    raffle_id: &str,
    entrant_id: &str,
) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    let removal = match raffle.state.removal() {
        Some(Removal::Delete) if entrants::delete(&app.db, raffle_id, entrant_id).await? => {
            Some(Removal::Delete)
        }
        // Nothing deleted: frozen since the read above (then refused), or no
        // such entrant (nothing to do).
        Some(Removal::Delete) => match raffles::get(&app.db, raffle_id).await? {
            Some(r) if r.state.removal().is_none() => {
                return Err(Refusal::NotRemovable.into());
            }
            _ => None,
        },
        Some(Removal::Erase) => entrants::erase(&app.db, raffle_id, Some(entrant_id))
            .await?
            .then_some(Removal::Erase),
        None => return Err(Refusal::NotRemovable.into()),
    };
    match removal {
        Some(Removal::Delete) => {
            audit::record(&app.db, raffle_id, owner, "remove_entrant", entrant_id).await?;
        }
        Some(Removal::Erase) => {
            audit::record(&app.db, raffle_id, owner, "erase_entrant", entrant_id).await?;
        }
        None => return Ok(()),
    }
    app.live_updates.notify(raffle_id).await;
    Ok(())
}

/// Erases every entrant's personal data of a finished raffle now, and ends
/// its live channel.
pub async fn purge(app: &App, owner: &str, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if raffle.state.apply(lifecycle::Command::Purge).is_none() {
        return Err(Refusal::NotPurgeable.into());
    }
    entrants::erase(&app.db, raffle_id, None).await?;
    raffles::mark_purged(&app.db, raffle_id).await?;
    audit::record(&app.db, raffle_id, owner, "purge", "").await?;
    // Destroying the channel publishes the final (erased) view first.
    app.live_updates.end(raffle_id).await?;
    Ok(())
}

/// Ends live updates of a finished raffle: phones stop listening and the
/// raffle's live channel is destroyed. Safe to repeat (the channel is
/// destroyed again).
pub async fn end_live(app: &App, owner: &str, raffle_id: &str) -> Result<()> {
    let raffle = owned(app, owner, raffle_id).await?;
    if !raffle.state.is_over() {
        return Err(Refusal::NotOver.into());
    }
    raffles::end_live(&app.db, raffle_id).await?;
    if raffle.live_ended_at.is_none() {
        audit::record(&app.db, raffle_id, owner, "end_live", "").await?;
    }
    app.live_updates.end(raffle_id).await?;
    Ok(())
}
