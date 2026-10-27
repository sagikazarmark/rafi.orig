//! What the organizer reads (the admin page, the winners' export) and what
//! anyone reads to verify a raffle.

use std::collections::HashMap;

use rafi_api::{AdminActions, AdminEntrant, Prize, RaffleAdmin, Retention, StartStep, VerifyData};
use rafi_lifecycle::{
    self as lifecycle, RaffleState,
    settings::{self, ErasureDates},
    start::{self, Start},
};
use rafi_raffle::{ItemId, Outcome, Raffle};
use rafi_store::{RaffleRow, audit, raffles, view::Full};

use crate::{
    Actor, App, Error, Result, ids,
    raffles::{missing_for_open, settings_of},
    view,
};

/// Everything the admin page shows of raffle `id`, from one read.
pub async fn admin(app: &App, owner: &str, id: &str) -> Result<RaffleAdmin> {
    let full = Full::read(&app.db, id).await?.ok_or(Error::NotFound)?;
    if full.raffle.owner_id != owner {
        return Err(Error::NotFound);
    }
    let now = ids::now();
    let view = view::of(app, &full)?;
    let raffle = &full.raffle;
    let retention = retention_of(app, raffle, now);
    let entrants = admin_entrants(&full, full.drawn.as_ref(), retention.winners_erase_at);
    let held_winners = entrants.iter().filter(|e| e.held_until.is_some()).count();
    let personal_data_left = full.entrants.iter().any(|e| !e.erased);
    let actions = AdminActions {
        start: start::next(raffle.state, raffle.manifest.is_some(), raffle.opened())
            .ok()
            .map(|step| match step {
                Start::Freeze { .. } => StartStep::Freeze,
                Start::WriteManifest { .. } | Start::Resend => StartStep::Retry,
                Start::Started => StartStep::Started,
            }),
        draw: raffle.state.frozen_or_later(),
        export: raffle.state.frozen_or_later(),
        purge: raffle.state.apply(lifecycle::Command::Purge).is_some()
            && (raffle.state == RaffleState::Finished || personal_data_left),
        edit_rules: settings::rules_editable(full.entrants.iter().map(|e| e.source)),
        removal: raffle.state.removal(raffle.past_erase_date(now)),
        deliver_all: raffle.state.is_over() && held_winners > 0,
    };
    Ok(RaffleAdmin {
        id: raffle.id.clone(),
        settings: settings_of(raffle),
        prizes: full
            .prizes
            .iter()
            .map(|i| Prize {
                id: i.id.clone(),
                name: i.name.clone(),
                sponsor: i.sponsor.clone(),
                description: i.description.clone(),
                qty: i.qty.max(0) as u32,
                position: i.position,
            })
            .collect(),
        entrants,
        view,
        public_url: app.policy.public_url.clone(),
        actions,
        retention: Retention {
            held_winners,
            ..retention
        },
        missing_for_open: missing_for_open(raffle)
            .into_iter()
            .map(str::to_owned)
            .collect(),
    })
}

/// When `raffle`'s personal data is erased, as of `now`.
fn retention_of(app: &App, raffle: &RaffleRow, now: i64) -> Retention {
    let hold = app.policy.retention.winner_hold_days;
    let (erase_date, winners_erase_date) =
        match ErasureDates::of(&raffle.event_date, raffle.retention_days, hold) {
            Some(dates) => (dates.erase.long(), dates.winners_erase.long()),
            None => (String::new(), String::new()),
        };
    let past_event = raffle.event_ends_at <= now;
    Retention {
        erase_at: raffle.erase_at,
        erase_date,
        winners_erase_at: app.policy.retention.winners_erase_at(raffle.erase_at),
        winners_erase_date,
        held_winners: 0,
        past_event,
        past_erase_date: raffle.past_erase_date(now),
        unfinished_past_event: past_event && raffle.state.drawable(),
        expires_unfrozen: past_event && raffle.state.expires(),
        default_days: app.policy.retention.days,
        max_days: app.policy.retention.max_days,
        winner_hold_days: hold,
    }
}

/// The entrants as the organizer sees them, with what the raffle made of
/// each. A claimed winner whose prize isn't delivered is held until
/// `winners_erase_at`.
fn admin_entrants(full: &Full, drawn: Option<&Raffle>, winners_erase_at: i64) -> Vec<AdminEntrant> {
    let outcomes = drawn.map(Raffle::entrant_outcomes).unwrap_or_default();
    let item_names: HashMap<ItemId, String> = drawn
        .map(|s| {
            s.items()
                .iter()
                .map(|item| (item.id.clone(), item.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    full.entrants
        .iter()
        .map(|e| {
            let outcome = outcomes.get(e.id.as_str());
            let deliverable = lifecycle::raffle::deliverable(drawn, &e.id);
            AdminEntrant {
                outcome: outcome.map(|o| o.0),
                prize: outcome.and_then(|o| item_names.get(&o.1).cloned()),
                id: e.id.clone(),
                display_name: e.display_name.clone(),
                email: e.email_raw.clone(),
                source: e.source,
                hidden: e.hidden,
                claim_code: e.claim_code.clone(),
                created_at: e.created_at,
                delivered: e.delivered_at.is_some(),
                deliverable,
                held_until: (deliverable && e.delivered_at.is_none() && !e.erased)
                    .then_some(winners_erase_at),
                erased: e.erased,
            }
        })
        .collect()
}

/// The drawn entrants as CSV: who won what, and how to reach them. Every
/// export is audited (who, when, how many rows), never stored.
pub async fn export_csv(app: &App, actor: Actor<'_>, id: &str) -> Result<String> {
    let full = Full::read(&app.db, id).await?.ok_or(Error::NotFound)?;
    if full.raffle.owner_id != actor.owner {
        return Err(Error::NotFound);
    }
    let hold = app.policy.retention.winners_erase_at(full.raffle.erase_at);
    fn esc(s: &str) -> String {
        // Quote, and neutralize spreadsheet formula injection.
        let s = if s.starts_with(['=', '+', '-', '@']) {
            format!("'{s}")
        } else {
            s.to_owned()
        };
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    let mut out =
        String::from("entrant_id,display_name,email,source,claim_code,outcome,prize,delivered\n");
    let mut rows = 0;
    for e in admin_entrants(&full, full.drawn.as_ref(), hold)
        .iter()
        .filter(|e| e.outcome.is_some())
    {
        rows += 1;
        let outcome = e.outcome.map_or("", Outcome::as_str);
        out.push_str(
            &[
                esc(&e.id),
                esc(&e.display_name),
                esc(e.email.as_deref().unwrap_or("")),
                esc(e.source.as_str()),
                esc(e.claim_code.as_deref().unwrap_or("")),
                esc(outcome),
                esc(e.prize.as_deref().unwrap_or("")),
                esc(if e.delivered { "yes" } else { "no" }),
            ]
            .join(","),
        );
        out.push('\n');
    }
    audit::record(&app.db, id, actor, "export_csv", &rows.to_string()).await?;
    Ok(out)
}

/// Everything needed to verify the draws of raffle `slug`, once its
/// manifest is published. One snapshot ([`Full::read`]): the names, the
/// events and the version agree, so the `ETag` built from `v` is honest.
pub async fn verify_data(app: &App, slug: &str) -> Result<VerifyData> {
    let id = raffles::by_slug(&app.db, slug)
        .await?
        .ok_or(Error::NotFound)?
        .id;
    let full = Full::read(&app.db, &id).await?.ok_or(Error::NotFound)?;
    let raffle = full.raffle;
    let (Some(manifest), Some(manifest_hash)) = (raffle.manifest, raffle.manifest_hash) else {
        return Err(Error::NotFound);
    };
    // Erased entrants have no name left: the page shows the start of their ID.
    let names = full
        .entrants
        .into_iter()
        .filter(|e| !e.erased)
        .map(|e| {
            let name = rafi_live_client::public_name(
                &e.id,
                Some((!e.hidden).then_some(e.display_name.as_str())),
            );
            (e.id, name)
        })
        .collect();
    Ok(VerifyData {
        format: rafi_api::VERIFY_FORMAT,
        v: raffle.version.max(0) as u64,
        title: raffle.title,
        state: raffle.state,
        test_mode: raffle.test_mode,
        events: full.drawn.map(|s| s.events().to_vec()).unwrap_or_default(),
        manifest,
        manifest_hash,
        names,
    })
}
