//! Raffles: creating, configuring, copying, deleting and moving them
//! through the lifecycle.

use rafi_api::{LifecycleCommand, RaffleDefaults, RaffleSettings, RaffleSummary};
use rafi_lifecycle::{self as lifecycle, dates::Date, settings};
use rafi_store::{RaffleRow, audit, defaults, entrants, prizes, raffles};

use crate::{App, Refusal, Result, ids, legal, owned};

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

/// The day a raffle is drawn, as the organizer gives it: the calendar day
/// (`YYYY-MM-DD`), and when it ends where they are (their browser says;
/// `None` for UTC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDate {
    pub date: String,
    pub ends_at: Option<i64>,
}

impl EventDate {
    /// Today, in UTC: for callers with no organizer's browser to ask.
    pub fn today() -> Self {
        Self {
            date: Date::of_utc(ids::now()).iso(),
            ends_at: None,
        }
    }
}

/// `settings` as they are stored, checked against the installation's
/// retention policy.
fn normalized<'a>(app: &App, settings: settings::Settings<'a>) -> Result<settings::Settings<'a>> {
    let max = app.policy.retention.max_days;
    Ok(settings
        .normalize(max)
        .map_err(|e| Refusal::settings(e, max))?)
}

/// A new draft raffle titled `title` on `event`, with a copy of the owner's
/// defaults; returns its ID. The owner must have accepted the current terms.
pub async fn create(app: &App, owner: &str, title: &str, event: &EventDate) -> Result<String> {
    legal::require_terms(app, owner).await?;
    let title = settings::title(title).ok_or(Refusal::Title)?;
    let d = defaults::get(&app.db, owner).await?.unwrap_or_default();
    let normalized = normalized(
        app,
        settings::Settings {
            title,
            description: "",
            organizer_name: &d.organizer_name,
            contact_email: &d.contact_email,
            privacy_email: &d.privacy_email,
            privacy_text: &d.privacy_text,
            privacy_url: &d.privacy_url,
            rules: &d.rules,
            event_date: &event.date,
            event_ends_at: event.ends_at,
            retention_days: d.retention_days.unwrap_or(app.policy.retention.days),
        },
    )?;
    let settings = stored(normalized, false);
    let id = ids::id();
    for _ in 0..8 {
        let res = raffles::insert_with(
            &app.db,
            &raffles::NewRaffle {
                id: &id,
                owner_id: owner,
                slug: &ids::slug(title),
                join_code: &ids::code(4),
                title,
            },
            &settings,
            &[],
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

/// What `owner`'s new raffles start with: empty until they save some.
pub async fn get_defaults(app: &App, owner: &str) -> Result<RaffleDefaults> {
    let d = defaults::get(&app.db, owner).await?.unwrap_or_default();
    Ok(RaffleDefaults {
        organizer_name: d.organizer_name,
        contact_email: d.contact_email,
        privacy_email: d.privacy_email,
        privacy_text: d.privacy_text,
        privacy_url: d.privacy_url,
        rules: d.rules,
        retention_days: d.retention_days,
    })
}

/// Saves what `owner`'s new raffles start with. Existing raffles keep theirs.
pub async fn save_defaults(app: &App, owner: &str, d: RaffleDefaults) -> Result<()> {
    let max = app.policy.retention.max_days;
    let d = settings::Defaults {
        organizer_name: &d.organizer_name,
        contact_email: &d.contact_email,
        privacy_email: &d.privacy_email,
        privacy_text: &d.privacy_text,
        privacy_url: &d.privacy_url,
        rules: &d.rules,
        retention_days: d.retention_days,
    }
    .normalize(max)
    .map_err(|e| Refusal::settings(e, max))?;
    defaults::save(
        &app.db,
        owner,
        &defaults::Defaults {
            organizer_name: d.organizer_name.to_owned(),
            contact_email: d.contact_email.to_owned(),
            privacy_email: d.privacy_email.to_owned(),
            privacy_text: d.privacy_text.to_owned(),
            privacy_url: d.privacy_url.to_owned(),
            rules: d.rules.to_owned(),
            retention_days: d.retention_days,
        },
    )
    .await?;
    Ok(())
}

pub fn settings_of(r: &RaffleRow) -> RaffleSettings {
    RaffleSettings {
        title: r.title.clone(),
        description: r.description.clone(),
        organizer_name: r.organizer_name.clone(),
        contact_email: r.contact_email.clone(),
        privacy_email: r.privacy_email.clone(),
        privacy_text: r.privacy_text.clone(),
        privacy_url: r.privacy_url.clone(),
        rules: r.rules.clone(),
        event_date: r.event_date.clone(),
        event_ends_at: Some(r.event_ends_at),
        retention_days: r.retention_days,
        test_mode: r.test_mode,
    }
}

/// The settings of a row, as the rules see them.
fn fixed_of(r: &RaffleRow) -> settings::Fixed<'_> {
    settings::Fixed {
        rules: &r.rules,
        event_date: &r.event_date,
        retention_days: r.retention_days,
        privacy_text: &r.privacy_text,
        privacy_url: &r.privacy_url,
    }
}

/// Saves a raffle's settings. Switching test mode off removes the raffle's
/// fake entrants in the same write. A fixed setting (the rules, the event
/// date, the retention period, the privacy text or link) can't change once
/// an entrant accepted the rules: the refusal names which.
pub async fn update_settings(app: &App, owner: &str, id: &str, s: RaffleSettings) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let normalized = normalized(
        app,
        settings::Settings {
            title: &s.title,
            description: &s.description,
            organizer_name: &s.organizer_name,
            contact_email: &s.contact_email,
            privacy_email: &s.privacy_email,
            privacy_text: &s.privacy_text,
            privacy_url: &s.privacy_url,
            rules: &s.rules,
            event_date: &s.event_date,
            // The stored instant, unless the day itself changes: a browser in
            // another timezone saving unrelated settings moves nothing.
            event_ends_at: if s.event_date.trim() == raffle.event_date {
                Some(raffle.event_ends_at)
            } else {
                s.event_ends_at
            },
            retention_days: s.retention_days,
        },
    )?;
    if raffle.test_mode != s.test_mode && !raffle.state.test_mode_editable() {
        return Err(Refusal::TestModeLocked.into());
    }
    // For the audit log: how many fakes switching test mode off removes.
    let fakes = if raffle.test_mode && !s.test_mode {
        entrants::count_fakes(&app.db, id).await?
    } else {
        0
    };
    let write = raffles::update_settings(&app.db, id, &stored(normalized, s.test_mode)).await?;
    if !write.written {
        // Test mode changed after a freeze since the read above, or a fixed
        // setting after someone accepted the rules.
        let current = raffles::get(&app.db, id).await?;
        let frozen = current
            .as_ref()
            .is_some_and(|r| r.test_mode != s.test_mode && !r.state.test_mode_editable());
        return Err(if frozen {
            Refusal::TestModeLocked
        } else {
            let changed = current
                .as_ref()
                .map(|r| normalized.fixed().changed_from(&fixed_of(r)))
                .unwrap_or_default();
            Refusal::SettingsFixed(changed)
        }
        .into());
    }
    if write.fakes_removed {
        audit::record(&app.db, id, owner, "remove_fakes", &fakes.to_string()).await?;
    }
    // Nothing phones see: the participant view has no settings, and names
    // only of drawn entrants (fakes are never drawn outside test mode).
    Ok(())
}

/// Normalized settings, as the store writes them.
fn stored(s: settings::Settings<'_>, test_mode: bool) -> raffles::Settings<'_> {
    raffles::Settings {
        title: s.title,
        description: s.description,
        organizer_name: s.organizer_name,
        contact_email: s.contact_email,
        privacy_email: s.privacy_email,
        privacy_text: s.privacy_text,
        privacy_url: s.privacy_url,
        rules: s.rules,
        event_date: s.event_date,
        event_ends_at: s
            .event_ends_at
            .expect("normalized settings resolve the end of the day"),
        retention_days: s.retention_days,
        erase_at: s.erase_at(),
        test_mode,
    }
}

pub async fn delete(app: &App, owner: &str, id: &str) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let drawn = raffles::drawn(&app.db, &raffle).await?;
    if !lifecycle::raffle::deletable(raffle.state, drawn.as_ref()) {
        return Err(Refusal::DrawPending.into());
    }
    raffles::delete(&app.db, id).await?;
    audit::record(&app.db, id, owner, "delete", "").await?;
    // Best effort: the raffle is gone either way, and its channel finds nothing to show.
    if let Err(e) = app.live_updates.end(id).await {
        tracing::warn!(raffle_id = id, error = %e, "ending the live channel of a deleted raffle failed");
    }
    Ok(())
}

/// Copies settings and prizes into a new draft raffle (no entrants) on
/// `event` (the organizer's today, as a rule), in one write: a copy is whole
/// or not there. The owner must have accepted the current terms.
pub async fn duplicate(app: &App, owner: &str, id: &str, event: &EventDate) -> Result<String> {
    legal::require_terms(app, owner).await?;
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
    // The copy is on `event`, with the original's period.
    let normalized = normalized(
        app,
        settings::Settings {
            title: &title,
            description: &raffle.description,
            organizer_name: &raffle.organizer_name,
            contact_email: &raffle.contact_email,
            privacy_email: &raffle.privacy_email,
            privacy_text: &raffle.privacy_text,
            privacy_url: &raffle.privacy_url,
            rules: &raffle.rules,
            event_date: &event.date,
            event_ends_at: event.ends_at,
            retention_days: raffle.retention_days.min(app.policy.retention.max_days),
        },
    )?;
    let settings = stored(normalized, raffle.test_mode);
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

/// Moves the raffle through its lifecycle. Registration opens only once the
/// privacy notice can name the organizer and a way to reach them
/// ([`settings::missing_for_open`]).
pub async fn lifecycle(app: &App, owner: &str, id: &str, action: LifecycleCommand) -> Result<()> {
    let raffle = owned(app, owner, id).await?;
    let command = match action {
        LifecycleCommand::Publish => lifecycle::Command::Publish,
        LifecycleCommand::Unpublish => lifecycle::Command::Unpublish,
        LifecycleCommand::Open => lifecycle::Command::Open,
        LifecycleCommand::Close => lifecycle::Command::Close,
    };
    let from = raffle.state;
    let to = from
        .apply(command)
        .ok_or(Refusal::Lifecycle { command, from })?;
    if command == lifecycle::Command::Open {
        let missing = missing_for_open(&raffle);
        if !missing.is_empty() {
            return Err(Refusal::OrganizerMissing(missing).into());
        }
    }
    if !raffles::transition(&app.db, id, from, to).await? {
        return Err(Refusal::Changed.into());
    }
    audit::record(&app.db, id, owner, to.as_str(), "").await?;
    app.live_updates.notify(id).await;
    Ok(())
}

/// What `raffle` still needs before registration can open.
pub fn missing_for_open(raffle: &RaffleRow) -> Vec<&'static str> {
    settings::missing_for_open(
        &raffle.organizer_name,
        &raffle.contact_email,
        &raffle.privacy_email,
    )
}
