//! Raffles: creating, configuring, copying, deleting and moving them
//! through the lifecycle.

use rafi_lifecycle::{self as raffle, settings};
use rafi_model::{Lifecycle, RaffleSettings, RaffleSummary};
use rafi_store::{RaffleRow, audit, prizes, raffles};

use crate::{App, Refusal, Result, ids, owned};

pub async fn list(app: &App, owner: &str) -> Result<Vec<RaffleSummary>> {
    Ok(raffles::list_by_owner(&app.db, owner)
        .await?
        .into_iter()
        .map(|r| RaffleSummary {
            state: r.state,
            id: r.id,
            slug: r.slug,
            title: r.title,
            entrant_count: r.entrant_count,
            test_mode: r.test_mode,
            created_at: r.created_at,
        })
        .collect())
}

/// A new draft raffle titled `title`; returns its ID.
pub async fn create(app: &App, owner: &str, title: &str) -> Result<String> {
    let title = settings::title(title).ok_or(Refusal::Title)?;
    let id = ids::id();
    for _ in 0..8 {
        let res = raffles::insert(
            &app.db,
            &raffles::NewRaffle {
                id: &id,
                owner_id: owner,
                slug: &ids::slug(title),
                join_code: &ids::code(4),
                title,
            },
        )
        .await;
        match res {
            Ok(_) => {
                audit::record(&app.db, &id, owner, "create", title).await?;
                return Ok(id);
            }
            Err(e) if e.is_unique_violation() => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Refusal::JoinCodes.into())
}

pub fn settings_of(r: &RaffleRow) -> RaffleSettings {
    RaffleSettings {
        title: r.title.clone(),
        description: r.description.clone(),
        lang: r.lang.clone(),
        organizer_name: r.organizer_name.clone(),
        contact_email: r.contact_email.clone(),
        rules: r.rules.clone(),
        event_date: r.event_date.clone(),
        min_age: r.min_age.max(0) as u32,
        test_mode: r.test_mode,
    }
}

pub async fn update_settings(app: &App, owner: &str, id: &str, s: RaffleSettings) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let normalized = settings::Settings {
        title: &s.title,
        description: &s.description,
        lang: &s.lang,
        organizer_name: &s.organizer_name,
        contact_email: &s.contact_email,
        rules: &s.rules,
        event_date: &s.event_date,
        min_age: s.min_age,
    }
    .normalize()
    .map_err(Refusal::from)?;
    if raffle.test_mode != s.test_mode && !raffle.state.test_mode_editable() {
        return Err(Refusal::TestModeLocked.into());
    }
    raffles::update_settings(&app.db, id, &stored(normalized, s.test_mode)).await?;
    // Nothing phones see: the participant view has no settings.
    Ok(())
}

/// Normalized settings, as the store writes them.
fn stored(s: settings::Settings<'_>, test_mode: bool) -> raffles::Settings<'_> {
    raffles::Settings {
        title: s.title,
        description: s.description,
        lang: s.lang,
        organizer_name: s.organizer_name,
        contact_email: s.contact_email,
        rules: s.rules,
        event_date: s.event_date,
        min_age: s.min_age,
        test_mode,
    }
}

pub async fn delete(app: &App, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    if !raffle::drawing::deletable(raffle.state, raffle.drawing()?.as_ref()) {
        return Err(Refusal::DrawPending.into());
    }
    raffles::delete(&app.db, id).await?;
    audit::record(&app.db, id, owner, "delete", "").await?;
    // Best effort: the raffle is gone either way, and its channel finds nothing to show.
    if let Err(e) = app.notifier.end(id).await {
        tracing::warn!(raffle_id = id, error = %e, "ending the live channel of a deleted raffle failed");
    }
    Ok(())
}

/// Copies settings and prizes into a new draft raffle (no entrants), in one
/// write: a copy is whole or not there.
pub async fn duplicate(app: &App, owner: &str, id: &str) -> Result<String> {
    let raffle = owned(app, owner, id).await?;
    let title = settings::copy_title(&raffle.title);
    let prizes = prizes::list(&app.db, id).await?;
    let copies: Vec<(String, prizes::PrizeFields)> = prizes
        .iter()
        .map(|prize| {
            (
                ids::code(6).to_lowercase(),
                prizes::PrizeFields {
                    name: &prize.name,
                    sponsor: &prize.sponsor,
                    description: &prize.description,
                    qty: prize.qty.max(0) as u32,
                    position: prize.position,
                },
            )
        })
        .collect();
    // The original's settings, as stored: normalized when they were written.
    let min_age = u32::try_from(raffle.min_age).unwrap_or(0);
    let settings = raffles::Settings {
        title: &title,
        description: &raffle.description,
        lang: settings::lang(&raffle.lang),
        organizer_name: &raffle.organizer_name,
        contact_email: &raffle.contact_email,
        rules: &raffle.rules,
        event_date: &raffle.event_date,
        min_age: min_age.min(settings::MAX_MIN_AGE),
        test_mode: raffle.test_mode,
    };
    let new_id = ids::id();
    for _ in 0..8 {
        let res = raffles::insert_with(
            &app.db,
            &raffles::NewRaffle {
                id: &new_id,
                owner_id: owner,
                slug: &ids::slug(&title),
                join_code: &ids::code(4),
                title: &title,
            },
            &settings,
            &copies,
        )
        .await;
        match res {
            Ok(()) => {
                audit::record(&app.db, &new_id, owner, "create", &title).await?;
                return Ok(new_id);
            }
            Err(e) if e.is_unique_violation() => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Refusal::JoinCodes.into())
}

pub async fn lifecycle(app: &App, owner: &str, id: &str, action: Lifecycle) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let command = match action {
        Lifecycle::Publish => raffle::Command::Publish,
        Lifecycle::Unpublish => raffle::Command::Unpublish,
        Lifecycle::Open => raffle::Command::Open,
        Lifecycle::Close => raffle::Command::Close,
    };
    let from = raffle.state;
    let to = from
        .apply(command)
        .ok_or(Refusal::Lifecycle { command, from })?;
    if !raffles::transition(&app.db, id, from, to).await? {
        return Err(Refusal::Changed.into());
    }
    audit::record(&app.db, id, owner, to.as_str(), "").await?;
    app.notifier.notify(id).await;
    Ok(())
}
