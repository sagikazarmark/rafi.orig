//! Builds the public [`RaffleView`] from the database.

use rafi_drawing::BeaconSource;
use std::collections::HashMap;

use rafi_core::raffle::RaffleState;
use rafi_drawing::drawing::{Event, ItemStatus, Outcome};
use rafi_store::view::Full;

use super::AppState;
use crate::model::{CurrentDraw, DrawView, ItemView, PrizeView, RaffleView};

/// Display name used when the organizer hid an entrant's name (the same on
/// phones).
pub use rafi_store::live::HIDDEN_NAME;

/// How long a new entrant's name waits before appearing on the screen.
pub const NAME_DELAY_SECS: i64 = 5;

pub fn join_url(app: &AppState, join_code: &str) -> String {
    format!("{}/j/{join_code}", app.cfg.public_url)
}

/// The full view of a raffle, or `None` if it doesn't exist. Its `v` is the
/// raffle's version.
pub async fn build(app: &AppState, raffle_id: &str) -> anyhow::Result<Option<RaffleView>> {
    let Some(Full {
        raffle,
        prizes,
        entrants,
    }) = Full::read(&app.db, raffle_id).await?
    else {
        return Ok(None);
    };
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

    let drawing = raffle.drawing();
    let prize_meta: HashMap<&str, (&str, &str)> = prizes
        .iter()
        .map(|i| (i.id.as_str(), (i.name.as_str(), i.sponsor.as_str())))
        .collect();

    let items: Vec<ItemView> = match &drawing {
        Some(s) => s
            .items
            .iter()
            .map(|u| ItemView {
                item_id: u.id.clone(),
                prize_id: rafi_core::prize::prize_id(&u.id).to_owned(),
                prize_name: u.name.clone(),
                status: u.status,
                display_name: u.entrant_id.as_deref().map(name_of),
                entrant_id: u.entrant_id.clone(),
            })
            .collect(),
        None => prizes
            .iter()
            .flat_map(|i| {
                (1..=u64::try_from(i.qty).unwrap_or(0)).map(move |n| ItemView {
                    item_id: rafi_core::prize::item_id(&i.id, n),
                    prize_id: i.id.clone(),
                    prize_name: i.name.clone(),
                    status: ItemStatus::Pending,
                    entrant_id: None,
                    display_name: None,
                })
            })
            .collect(),
    };

    // The raffle's prizes in order, with how many are still to be won.
    let mut prize_views: Vec<PrizeView> = Vec::new();
    for u in &items {
        let (name, sponsor) = prize_meta
            .get(u.prize_id.as_str())
            .copied()
            .unwrap_or((u.prize_name.as_str(), ""));
        let open = matches!(u.status, ItemStatus::Pending | ItemStatus::Drawn);
        match prize_views.iter_mut().find(|p| p.prize_id == u.prize_id) {
            Some(p) => {
                p.qty += 1;
                p.left += open as u32;
            }
            None => prize_views.push(PrizeView {
                prize_id: u.prize_id.clone(),
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
    if let Some(s) = &drawing {
        let mut outcomes: HashMap<u64, Outcome> = HashMap::new();
        for e in &s.events {
            if let Event::Resolve { k, outcome } = e {
                outcomes.insert(*k, *outcome);
            }
        }
        let item_name: HashMap<&str, &str> = s
            .items
            .iter()
            .map(|u| (u.id.as_str(), u.name.as_str()))
            .collect();
        for e in &s.events {
            match e {
                Event::Draw {
                    k,
                    item_id,
                    entrant_id,
                    pass,
                    mode,
                    round,
                    ..
                } => {
                    last_failure = None;
                    draws.push(DrawView {
                        k: *k,
                        item_id: item_id.clone(),
                        prize_name: item_name
                            .get(item_id.as_str())
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
                // A stalled draw stays pending on its round; an abandoned one is over.
                Event::Stall { reason, .. } => last_failure = Some(reason.clone()),
                Event::Abandon { round, reason, .. } => {
                    last_failure = Some(format!("round {round} abandoned ({reason})"))
                }
                Event::Announce { .. } => last_failure = None,
                _ => {}
            }
        }
        if let Some(p) = &s.pending {
            current = Some(CurrentDraw {
                k: p.k,
                item_id: p.item_id.clone(),
                prize_name: item_name
                    .get(p.item_id.as_str())
                    .copied()
                    .unwrap_or_default()
                    .to_owned(),
                round: p.round,
                round_time: p.round_time,
            });
        }
    }

    let state = raffle.state();
    // Labelled by the beacon source the raffle committed to (or will): only
    // a public drand chain is verifiable.
    let beacon = raffle
        .drawing()
        .map(|s| s.beacon)
        .unwrap_or(BeaconSource::Drand(app.chain));
    let public = beacon
        .drand_chain()
        .is_some_and(|c| rafi_drawing::drand::chains::by_hash(c).is_some());
    let engine = if public {
        "restate"
    } else {
        "restate (mock drand)"
    }
    .to_owned();
    Ok(Some(RaffleView {
        v: raffle.version.max(0) as u64,
        live: app.transport(&raffle.id, raffle.live_ended_at.is_some()),
        live_ended: raffle.live_ended_at.is_some(),
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
        prizes: prize_views,
        items,
        current,
        draws,
        last_failure,
        manifest_hash: raffle.manifest_hash.clone(),
        server_now_ms: crate::model::now_unix_ms(),
        test_mode: raffle.test_mode,
        engine,
        state,
    }))
}
