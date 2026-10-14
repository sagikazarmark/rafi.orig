//! What the organizer reads (the admin page, the winners' export) and what
//! anyone reads to verify a raffle.

use std::collections::HashMap;

use rafi_lifecycle::{
    self as lifecycle, RaffleState,
    start::{self, Start},
};
use rafi_model::{AdminActions, AdminEntrant, Prize, RaffleAdmin, StartStep, VerifyData};
use rafi_raffle::{ItemId, Outcome, Raffle};
use rafi_store::{entrants, raffles, view::Full};

use crate::{App, Error, Result, raffles::settings_of, view};

/// Everything the admin page shows of raffle `id`, from one read.
pub async fn admin(app: &App, owner: &str, id: &str) -> Result<RaffleAdmin> {
    let full = Full::read(&app.db, id).await?.ok_or(Error::NotFound)?;
    if full.raffle.owner_id != owner {
        return Err(Error::NotFound);
    }
    let drawn = full.raffle.drawn()?;
    let view = view::of(app, &full)?;
    let entrants = admin_entrants(&full, drawn.as_ref());
    let raffle = &full.raffle;
    let personal_data_left = full.entrants.iter().any(|e| !e.purged);
    let actions = AdminActions {
        start: start::next(
            raffle.state,
            raffle.manifest.is_some(),
            raffle.log.is_some(),
        )
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
    })
}

/// The entrants as the organizer sees them, with what the raffle made of
/// each.
fn admin_entrants(full: &Full, drawn: Option<&Raffle>) -> Vec<AdminEntrant> {
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
            AdminEntrant {
                outcome: outcome.map(|o| o.0),
                prize: outcome.and_then(|o| item_names.get(&o.1).cloned()),
                id: e.id.clone(),
                display_name: e.display_name.clone(),
                email: e.email_raw.clone(),
                source: e.source,
                hidden: e.hidden,
                claim_code: e.claim_code.clone(),
                marketing: e.consent_marketing,
                created_at: e.created_at,
                delivered: e.delivered_at.is_some(),
                deliverable: lifecycle::raffle::deliverable(drawn, &e.id),
                purged: e.purged,
            }
        })
        .collect()
}

/// The drawn entrants as CSV: who won what, and how to reach them.
pub async fn export_csv(app: &App, owner: &str, id: &str) -> Result<String> {
    let full = Full::read(&app.db, id).await?.ok_or(Error::NotFound)?;
    if full.raffle.owner_id != owner {
        return Err(Error::NotFound);
    }
    let drawn = full.raffle.drawn()?;
    fn esc(s: &str) -> String {
        // Quote, and neutralize spreadsheet formula injection.
        let s = if s.starts_with(['=', '+', '-', '@']) {
            format!("'{s}")
        } else {
            s.to_owned()
        };
        format!("\"{}\"", s.replace('"', "\"\""))
    }
    let mut out = String::from(
        "entrant_id,display_name,email,source,claim_code,outcome,prize,delivered,marketing_consent\n",
    );
    for e in admin_entrants(&full, drawn.as_ref())
        .iter()
        .filter(|e| e.outcome.is_some())
    {
        let outcome = match e.outcome {
            Some(Outcome::Claimed) => "claimed",
            Some(Outcome::Drawn) => "drawn",
            Some(Outcome::NotHere) => "not_here",
            Some(Outcome::Declined) => "declined",
            None => "",
        };
        out.push_str(
            &[
                esc(&e.id),
                esc(&e.display_name),
                esc(e.email.as_deref().unwrap_or("")),
                esc(e.source.as_str()),
                esc(&e.claim_code),
                esc(outcome),
                esc(e.prize.as_deref().unwrap_or("")),
                esc(if e.delivered { "yes" } else { "no" }),
                esc(if e.marketing { "yes" } else { "no" }),
            ]
            .join(","),
        );
        out.push('\n');
    }
    Ok(out)
}

/// Everything needed to verify the draws of raffle `slug`, once its
/// manifest is published.
pub async fn verify_data(app: &App, slug: &str) -> Result<VerifyData> {
    let raffle = raffles::by_slug(&app.db, slug)
        .await?
        .ok_or(Error::NotFound)?;
    let (Some(manifest), Some(manifest_hash)) =
        (raffle.manifest.clone(), raffle.manifest_hash.clone())
    else {
        return Err(Error::NotFound);
    };
    let names = entrants::list(&app.db, &raffle.id)
        .await?
        .into_iter()
        .map(|e| {
            let name = rafi_live_client::public_name(
                &e.id,
                Some((!e.hidden).then_some(e.display_name.as_str())),
            );
            (e.id, name)
        })
        .collect();
    Ok(VerifyData {
        title: raffle.title.clone(),
        state: raffle.state,
        events: raffle
            .drawn()?
            .map(|s| s.events().to_vec())
            .unwrap_or_default(),
        manifest,
        manifest_hash,
        names,
    })
}
