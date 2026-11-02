//! Raffles: creating, configuring, copying, deleting and moving them
//! through the lifecycle.

use rafi_api::{LifecycleCommand, RaffleDefaults, RaffleSettings, RaffleSummary};
use rafi_engine::EngineError;
use rafi_lifecycle::{self as lifecycle, dates::Date, settings};
use rafi_raffle::{Decision, ErrorKind, RaffleId};
use rafi_store::{RaffleRow, audit, defaults, entrants, prizes, raffles};

use crate::{Actor, App, Refusal, Result, ids, owned};

/// The join code's length: six characters of [`crate::codes::ALPHABET`]
/// (31⁶ ≈ 887 million). Short enough to type from the projector when the QR
/// code won't scan; a purged raffle keeps its code, so the space only fills.
/// Lengthening it is a change here alone: the column is text, old codes stay
/// valid, and [`crate::codes::normalize`] accepts any grouping.
pub const JOIN_CODE_LEN: usize = 6;

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
            event_date: r.event_date,
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
    /// Today by `app`'s clock, in UTC: for callers with no organizer's
    /// browser to ask.
    pub fn today(app: &App) -> Self {
        Self {
            date: Date::of_utc(app.clock.now()).iso(),
            ends_at: None,
        }
    }
}

/// `settings` as they are stored `now`, checked against the installation's
/// retention policy.
fn normalized<'a>(
    app: &App,
    settings: settings::Settings<'a>,
    now: i64,
) -> Result<settings::Settings<'a>> {
    let max = app.policy.retention.max_days;
    Ok(settings
        .normalize(max, Date::of_utc(now))
        .map_err(|e| Refusal::settings(e, max))?)
}

/// A new draft raffle titled `title` on `event`, with a copy of the owner's
/// defaults; returns its ID.
pub async fn create(app: &App, actor: Actor<'_>, title: &str, event: &EventDate) -> Result<String> {
    let title = settings::title(title).ok_or(Refusal::Title)?;
    let now = app.clock.now();
    let d = defaults::get(&app.db, actor.owner)
        .await?
        .unwrap_or_default();
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
        now,
    )?;
    let settings = stored(normalized, false, d.registration);
    let id = ids::id();
    for _ in 0..8 {
        let res = raffles::insert_with(
            &app.db,
            &raffles::NewRaffle {
                id: &id,
                owner_id: actor.owner,
                slug: &ids::slug(title),
                join_code: &ids::code(JOIN_CODE_LEN),
                title,
            },
            &settings,
            &[],
            now,
        )
        .await;
        match res {
            Ok(_) => {
                audit::record(&app.db, &id, actor, audit::Action::Create, title, now).await?;
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
        registration: d.registration,
    })
}

/// Saves what `owner`'s new raffles start with. Existing raffles keep theirs.
pub async fn save_defaults(app: &App, actor: Actor<'_>, d: RaffleDefaults) -> Result<()> {
    let max = app.policy.retention.max_days;
    let registration = d.registration;
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
        actor.owner,
        &defaults::Defaults {
            organizer_name: d.organizer_name.to_owned(),
            contact_email: d.contact_email.to_owned(),
            privacy_email: d.privacy_email.to_owned(),
            privacy_text: d.privacy_text.to_owned(),
            privacy_url: d.privacy_url.to_owned(),
            rules: d.rules.to_owned(),
            retention_days: d.retention_days,
            registration,
        },
        app.clock.now(),
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
        registration: r.registration,
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
/// a real entrant is in, joined or added: the refusal names which. Registration
/// switches before the freeze, but not while it is open
/// ([`RaffleState::registration_editable`](lifecycle::RaffleState::registration_editable)).
pub async fn update_settings(
    app: &App,
    actor: Actor<'_>,
    id: &str,
    s: RaffleSettings,
) -> Result<()> {
    let raffle = owned(app, actor.owner, id).await?;
    let now = app.clock.now();
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
        now,
    )?;
    if raffle.test_mode != s.test_mode && !raffle.state.test_mode_editable() {
        return Err(Refusal::TestModeLocked.into());
    }
    if raffle.registration != s.registration && !raffle.state.registration_editable() {
        return Err(Refusal::RegistrationLocked.into());
    }
    // For the audit log: how many fakes switching test mode off removes.
    let fakes = if raffle.test_mode && !s.test_mode {
        entrants::count_fakes(&app.db, id).await?
    } else {
        0
    };
    let written = stored(normalized, s.test_mode, s.registration);
    let write = raffles::update_settings(&app.db, id, &written, now).await?;
    if !write.written {
        // Test mode changed after a freeze since the read above,
        // registration after it opened or froze, or a fixed setting after
        // a real entrant came in.
        let current = raffles::get(&app.db, id).await?;
        let frozen = current
            .as_ref()
            .is_some_and(|r| r.test_mode != s.test_mode && !r.state.test_mode_editable());
        let registration = current
            .as_ref()
            .is_some_and(|r| r.registration != s.registration && !r.state.registration_editable());
        return Err(if frozen {
            Refusal::TestModeLocked
        } else if registration {
            Refusal::RegistrationLocked
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
        audit::record(
            &app.db,
            id,
            actor,
            audit::Action::RemoveFakes,
            &fakes.to_string(),
            now,
        )
        .await?;
    }
    // What changed, by field name (never the values: the organizer's name
    // and contacts are personal data; the log says who changed what, when).
    let changed = changed_fields(&raffle, &written);
    if !changed.is_empty() {
        audit::record(
            &app.db,
            id,
            actor,
            audit::Action::Settings,
            &changed.join(","),
            now,
        )
        .await?;
    }
    // Nothing phones see: the participant view has no settings, and names
    // only of drawn entrants (fakes are never drawn outside test mode). The
    // projector's QR code goes with registration, though: tell it at once.
    if raffle.registration != s.registration {
        app.live_updates.notify(id).await;
    }
    Ok(())
}

/// The settings that differ between the row and what was written, by name.
fn changed_fields(before: &RaffleRow, after: &raffles::Settings<'_>) -> Vec<&'static str> {
    let mut changed = Vec::new();
    let mut diff = |name: &'static str, same: bool| {
        if !same {
            changed.push(name);
        }
    };
    diff("title", before.title == after.title);
    diff("description", before.description == after.description);
    diff(
        "organizer_name",
        before.organizer_name == after.organizer_name,
    );
    diff("contact_email", before.contact_email == after.contact_email);
    diff("privacy_email", before.privacy_email == after.privacy_email);
    diff("privacy_text", before.privacy_text == after.privacy_text);
    diff("privacy_url", before.privacy_url == after.privacy_url);
    diff("rules", before.rules == after.rules);
    diff("event_date", before.event_date == after.event_date);
    diff(
        "retention_days",
        before.retention_days == after.retention_days,
    );
    diff("test_mode", before.test_mode == after.test_mode);
    diff("registration", before.registration == after.registration);
    changed
}

/// Normalized settings, as the store writes them.
fn stored(s: settings::Settings<'_>, test_mode: bool, registration: bool) -> raffles::Settings<'_> {
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
        test_mode,
        registration,
    }
}

/// Deletes a raffle that hasn't drawn ([`lifecycle::raffle::deletable`]):
/// its entrants, prizes and log go with it. A frozen raffle the engine has
/// opened is finished there first, so the engine forgets it instead of
/// holding its state until the retention ceiling; if the engine can't be
/// reached, the raffle is deleted all the same (housekeeping asks again at
/// the ceiling, and finds nothing to write to).
///
/// A draw can be announced after the check (another organizer's Draw): the
/// engine then refuses the finish (a draw is pending) or queues it behind
/// that draw, and either way the raffle is refused as drawn. The delete
/// itself is guarded on no draw having been announced, so a draw the
/// engine stored meanwhile keeps the raffle too.
pub async fn delete(app: &App, actor: Actor<'_>, id: &str) -> Result<()> {
    let raffle = owned(app, actor.owner, id).await?;
    let drawn = raffles::drawn(&app.db, &raffle).await?;
    if !lifecycle::raffle::deletable(drawn.as_ref()) {
        return Err(Refusal::Drawn.into());
    }
    if drawn.is_some_and(|d| !d.is_finished())
        && let Ok(engine) = app.engine()
    {
        let command_id = format!("delete-{id}");
        match engine
            .decide(&RaffleId::new(id), &Decision::Finish, &command_id)
            .await
        {
            Ok(()) => {}
            // It holds a draw: one was announced since the check.
            Err(
                EngineError::Pending
                | EngineError::Rejected {
                    kind: ErrorKind::Conflict,
                    ..
                },
            ) => {
                return Err(Refusal::Drawn.into());
            }
            // It doesn't hold the raffle (never opened there, or forgotten).
            Err(EngineError::Rejected {
                kind: ErrorKind::NotFound,
                ..
            }) => {}
            Err(e @ EngineError::Rejected { .. }) => return Err(e.into()),
            Err(EngineError::Unavailable(e)) => {
                tracing::warn!(raffle_id = id, error = %format!("{e:#}"), "the engine couldn't be asked to forget a deleted raffle");
            }
        }
    }
    if !raffles::delete(&app.db, id).await? {
        return Err(Refusal::Drawn.into());
    }
    audit::record(
        &app.db,
        id,
        actor,
        audit::Action::Delete,
        "",
        app.clock.now(),
    )
    .await?;
    // Best effort: the raffle is gone either way, and its channel finds nothing to show.
    if let Err(e) = app.live_updates.end(id).await {
        tracing::warn!(raffle_id = id, error = %e, "ending the live channel of a deleted raffle failed");
    }
    Ok(())
}

/// Copies settings and prizes into a new draft raffle (no entrants) on
/// `event` (the organizer's today, as a rule), in one write: a copy is whole
/// or not there.
pub async fn duplicate(app: &App, actor: Actor<'_>, id: &str, event: &EventDate) -> Result<String> {
    let raffle = owned(app, actor.owner, id).await?;
    let now = app.clock.now();
    let title = settings::copy_title(&raffle.title);
    let prizes = prizes::list(&app.db, id).await?;
    let copies: Vec<(String, prizes::PrizeFields)> = prizes
        .iter()
        .map(|prize| {
            (
                ids::id(),
                prizes::PrizeFields {
                    name: &prize.name,
                    sponsor: &prize.sponsor,
                    description: &prize.description,
                    qty: prize.qty.max(0) as u32,
                    position: prize.position,
                    delivery: prize.delivery,
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
        now,
    )?;
    let settings = stored(normalized, raffle.test_mode, raffle.registration);
    let new_id = ids::id();
    for _ in 0..8 {
        let res = raffles::insert_with(
            &app.db,
            &raffles::NewRaffle {
                id: &new_id,
                owner_id: actor.owner,
                slug: &ids::slug(&title),
                join_code: &ids::code(JOIN_CODE_LEN),
                title: &title,
            },
            &settings,
            &copies,
            now,
        )
        .await;
        match res {
            Ok(()) => {
                audit::record(&app.db, &new_id, actor, audit::Action::Create, &title, now).await?;
                return Ok(new_id);
            }
            Err(e) if e.is_unique_violation() => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(Refusal::JoinCodes.into())
}

/// Moves the raffle through its lifecycle, as its registration setting
/// allows ([`lifecycle::RaffleState::apply`]): a raffle without
/// registration doesn't open. Registration opens only once the privacy
/// notice can name the organizer and a way to reach them
/// ([`settings::missing_for_open`]), before the erasure date.
pub async fn lifecycle(
    app: &App,
    actor: Actor<'_>,
    id: &str,
    action: LifecycleCommand,
) -> Result<()> {
    let raffle = owned(app, actor.owner, id).await?;
    let now = app.clock.now();
    let command = lifecycle::Command::from(action);
    let from = raffle.state;
    let to = match from.apply(command, raffle.registration) {
        Some(to) => to,
        None if from.apply(command, true).is_some() => return Err(Refusal::NoRegistration.into()),
        None => return Err(Refusal::Lifecycle { command, from }.into()),
    };
    if command == lifecycle::Command::Open {
        ready_for_entrants(&raffle, now)?;
    }
    // Guarded on the setting too: switched since the read, the move may
    // not hold any more.
    if !raffles::transition(&app.db, id, from, to, raffle.registration, now).await? {
        return Err(Refusal::Changed.into());
    }
    audit::record(&app.db, id, actor, audit::Action::To(to), "", now).await?;
    app.live_updates.notify(id).await;
    Ok(())
}

/// Whether `raffle` may take entrants for its draw `now`: what opening
/// registration checks, and what freezing from Published checks for a
/// raffle without registration, where no opening came first. The privacy
/// notice must name a controller and a way to reach them
/// ([`missing_for_open`]), and nobody enters a raffle that expires within
/// the hour (its erasure date passed).
pub(crate) fn ready_for_entrants(raffle: &RaffleRow, now: i64) -> Result<(), Refusal> {
    let fields = missing_for_open(raffle);
    if !fields.is_empty() {
        return Err(Refusal::OrganizerMissing {
            fields,
            registration: raffle.registration,
        });
    }
    if raffle.past_erase_date(now) {
        return Err(Refusal::ErasureDatePassed);
    }
    Ok(())
}

/// What `raffle` still needs before registration can open (or, without
/// registration, before the list can be frozen from Published).
pub fn missing_for_open(raffle: &RaffleRow) -> Vec<&'static str> {
    settings::missing_for_open(
        &raffle.organizer_name,
        &raffle.contact_email,
        &raffle.privacy_email,
    )
}
