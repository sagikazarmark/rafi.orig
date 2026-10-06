//! Builds the public [`RaffleView`] from the database.

use std::collections::HashMap;

use rafi_core::{
    raffle::RaffleState,
    state::{Event, Outcome, UnitStatus},
};

use super::{AppState, db};
use crate::model::{CurrentDraw, DrawView, PrizeView, RaffleView, UnitView};

/// Display name used when the organizer hid an entrant's name.
pub const HIDDEN_NAME: &str = "★ ★ ★";

/// How long a new entrant's name waits before appearing on the screen.
pub const NAME_DELAY_SECS: i64 = 5;

pub fn join_url(app: &AppState, join_code: &str) -> String {
    format!("{}/j/{join_code}", app.cfg.public_url)
}

pub async fn build(app: &AppState, raffle_id: &str) -> anyhow::Result<Option<RaffleView>> {
    let Some(raffle) = db::raffle(&app.db, raffle_id).await? else {
        return Ok(None);
    };
    let items = db::items(&app.db, raffle_id).await?;
    let entrants = db::entrants(&app.db, raffle_id).await?;
    let now = super::ids::now();

    let names: HashMap<&str, String> = entrants
        .iter()
        .map(|e| {
            let name = if e.hidden {
                HIDDEN_NAME.to_owned()
            } else {
                e.display_name.clone()
            };
            (e.id.as_str(), name)
        })
        .collect();
    let name_of = |id: &str| {
        names
            .get(id)
            .cloned()
            .unwrap_or_else(|| format!("#{}", &id[..id.len().min(6)]))
    };

    let recent_names = entrants
        .iter()
        .rev()
        .filter(|e| !e.hidden && !e.purged && e.created_at <= now - NAME_DELAY_SECS)
        .take(20)
        .map(|e| e.display_name.clone())
        .collect();

    let draw_state = raffle.draw_state();
    let item_meta: HashMap<&str, (&str, &str)> = items
        .iter()
        .map(|i| (i.id.as_str(), (i.name.as_str(), i.sponsor.as_str())))
        .collect();

    let units: Vec<UnitView> = match &draw_state {
        Some(s) => s
            .units
            .iter()
            .map(|u| UnitView {
                unit_id: u.unit_id.clone(),
                item_id: u.item_id.clone(),
                item_name: u.item_name.clone(),
                status: u.status,
                display_name: u.entrant_id.as_deref().map(name_of),
                entrant_id: u.entrant_id.clone(),
            })
            .collect(),
        None => items
            .iter()
            .flat_map(|i| {
                (1..=i.qty.max(0)).map(move |n| UnitView {
                    unit_id: super::service::unit_id(&i.id, n),
                    item_id: i.id.clone(),
                    item_name: i.name.clone(),
                    status: UnitStatus::Pending,
                    entrant_id: None,
                    display_name: None,
                })
            })
            .collect(),
    };

    // Prizes: original items in order, with how many are still to be won.
    let mut prizes: Vec<PrizeView> = Vec::new();
    for u in &units {
        let (name, sponsor) = item_meta
            .get(u.item_id.as_str())
            .copied()
            .unwrap_or((u.item_name.as_str(), ""));
        let open = matches!(u.status, UnitStatus::Pending | UnitStatus::Drawn);
        match prizes.iter_mut().find(|p| p.item_id == u.item_id) {
            Some(p) => {
                p.qty += 1;
                p.left += open as u32;
            }
            None => prizes.push(PrizeView {
                item_id: u.item_id.clone(),
                name: name.to_owned(),
                sponsor: sponsor.to_owned(),
                qty: 1,
                left: open as u32,
            }),
        }
    }

    let mut draws = Vec::new();
    let mut current = None;
    let mut last_failure = None;
    if let Some(s) = &draw_state {
        let mut outcomes: HashMap<u64, Outcome> = HashMap::new();
        for e in &s.events {
            if let Event::Resolve { k, outcome } = e {
                outcomes.insert(*k, *outcome);
            }
        }
        let unit_name: HashMap<&str, &str> = s
            .units
            .iter()
            .map(|u| (u.unit_id.as_str(), u.item_name.as_str()))
            .collect();
        for e in &s.events {
            match e {
                Event::Draw {
                    k,
                    unit_id,
                    entrant_id,
                    pass,
                    mode,
                    round,
                    ..
                } => {
                    last_failure = None;
                    draws.push(DrawView {
                        k: *k,
                        unit_id: unit_id.clone(),
                        item_name: unit_name
                            .get(unit_id.as_str())
                            .copied()
                            .unwrap_or_default()
                            .to_owned(),
                        entrant_id: entrant_id.clone(),
                        display_name: name_of(entrant_id),
                        outcome: outcomes.get(k).copied().unwrap_or(Outcome::Drawn),
                        pass: *pass,
                        mode: *mode,
                        round: *round,
                    });
                }
                Event::Fail { reason, .. } => last_failure = Some(reason.clone()),
                Event::Announce { .. } => last_failure = None,
                _ => {}
            }
        }
        if let Some(p) = &s.pending {
            current = Some(CurrentDraw {
                k: p.k,
                unit_id: p.unit_id.clone(),
                item_name: unit_name
                    .get(p.unit_id.as_str())
                    .copied()
                    .unwrap_or_default()
                    .to_owned(),
                round: p.round,
                round_time: p.round_time,
            });
        }
    }

    let state = raffle.state();
    // The app's drand settings only apply to the local engine; restate-rafi has its own.
    let engine = match (raffle.engine.as_str(), app.drand.is_mock()) {
        (e @ "local", true) => format!("{e} (mock drand)"),
        (e, _) => e.to_owned(),
    };
    Ok(Some(RaffleView {
        v: app.hub.next_version(),
        raffle_id: raffle.id.clone(),
        slug: raffle.slug.clone(),
        title: raffle.title.clone(),
        description: raffle.description.clone(),
        lang: raffle.lang.clone(),
        join_url: join_url(app, &raffle.join_code),
        join_code: raffle.join_code.clone(),
        entrant_count: entrants
            .iter()
            .filter(|e| !e.purged || state != RaffleState::Purged)
            .count(),
        recent_names,
        prizes,
        units,
        current,
        draws,
        last_failure,
        list_hash: raffle.list_hash.clone(),
        server_now_ms: crate::model::now_unix_ms(),
        test_mode: raffle.test_mode,
        engine,
        state,
    }))
}
