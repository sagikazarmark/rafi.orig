//! The full view of a raffle ([`RaffleView`]): what the projector and the
//! admin page show.

use std::collections::HashMap;

use rafi_api::{CurrentDraw, DrawView, ItemView, PrizeView, RaffleView};
use rafi_lifecycle::RaffleState;
use rafi_raffle::{BeaconSource, Event, ItemId, ItemStatus};
use rafi_store::view::Full;

use crate::App;

/// How long a new entrant's name waits before appearing on the screen.
pub const NAME_DELAY_SECS: i64 = 5;

pub fn join_url(app: &App, join_code: &str) -> String {
    format!("{}/j/{join_code}", app.policy.public_url)
}

/// The full view of a raffle, or `None` if it doesn't exist. Its `v` is the
/// raffle's version.
pub async fn build(app: &App, raffle_id: &str) -> anyhow::Result<Option<RaffleView>> {
    match Full::read(&app.db, raffle_id).await? {
        Some(full) => Ok(Some(of(app, &full, app.clock.now())?)),
        None => Ok(None),
    }
}

/// The full view of a raffle read as `full`, at `now`.
pub fn of(app: &App, full: &Full, now: i64) -> anyhow::Result<RaffleView> {
    let Full {
        raffle,
        prizes,
        entrants,
        drawn,
    } = full;

    // Erased entrants have no name left: screens show the start of their ID.
    let known: HashMap<&str, Option<&str>> = entrants
        .iter()
        .filter(|e| !e.erased)
        .map(|e| {
            let name = (!e.hidden).then_some(e.display_name.as_str());
            (e.id.as_str(), name)
        })
        .collect();
    let name_of = |id: &str| rafi_live_client::public_name(id, known.get(id).copied());

    let recent_names = entrants
        .iter()
        .rev()
        .filter(|e| !e.hidden && !e.erased && e.created_at <= now - NAME_DELAY_SECS)
        .take(20)
        .map(|e| e.display_name.clone())
        .collect();

    let prize_meta: HashMap<&str, (&str, &str)> = prizes
        .iter()
        .map(|i| (i.id.as_str(), (i.name.as_str(), i.sponsor.as_str())))
        .collect();

    let items: Vec<ItemView> = match drawn {
        Some(s) => s
            .items()
            .iter()
            .map(|u| ItemView {
                item: u.id.clone(),
                prize_id: rafi_lifecycle::raffle::prize_id(&u.id).to_owned(),
                prize_name: u.name.clone(),
                status: u.status(),
                display_name: u.winner().map(|e| name_of(e.as_str())),
                entrant: u.winner().cloned(),
            })
            .collect(),
        // Before the freeze: the items the raffle's prizes will become.
        None => rafi_lifecycle::raffle::items(prizes.iter().map(|i| {
            (
                i.id.as_str(),
                i.name.as_str(),
                u64::try_from(i.qty).unwrap_or(0),
            )
        }))
        .into_iter()
        .map(|item| ItemView {
            prize_id: rafi_lifecycle::raffle::prize_id(&item.id).to_owned(),
            item: item.id,
            prize_name: item.name,
            status: ItemStatus::Available,
            entrant: None,
            display_name: None,
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
        let open = matches!(u.status, ItemStatus::Available | ItemStatus::Drawn);
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
    if let Some(s) = drawn {
        let item_name = |id: &ItemId| s.item(id).map(|i| i.name.clone()).unwrap_or_default();
        draws = s
            .draws()
            .into_iter()
            .map(|d| DrawView {
                k: d.k,
                prize_name: item_name(&d.item),
                item: d.item,
                display_name: name_of(d.entrant.as_str()),
                entrant: d.entrant,
                outcome: d.outcome,
                pass: d.pass,
                round: d.round,
            })
            .collect();
        // The latest attempt's trouble: a stalled draw stays pending on its
        // round; an abandoned one is over. A new announcement or a draw
        // clears it.
        for e in s.events() {
            match e {
                Event::Stalled { reason, .. } => last_failure = Some(reason.clone()),
                Event::Abandoned { round, reason, .. } => {
                    last_failure = Some(format!("round {round} abandoned ({reason})"))
                }
                Event::Announced { .. } | Event::Drawn { .. } => last_failure = None,
                _ => {}
            }
        }
        if let Some(p) = s.pending() {
            current = Some(CurrentDraw {
                k: p.k,
                item: p.item.clone(),
                prize_name: item_name(&p.item),
                round: p.round,
                round_time: p.round_time,
            });
        }
    }

    let state = raffle.state;
    // Labelled by the beacon source the raffle committed to (or will): only
    // a public drand chain is verifiable.
    let verifiable = drawn
        .as_ref()
        .map_or(BeaconSource::Drand(app.chain), |s| *s.beacon())
        .publicly_verifiable();
    Ok(RaffleView {
        v: raffle.version.max(0) as u64,
        follow: app.follow(&raffle.id, raffle.live_ended_at.is_some()),
        live_ended: raffle.live_ended_at.is_some(),
        raffle_id: raffle.id.clone(),
        slug: raffle.slug.clone(),
        title: raffle.title.clone(),
        description: raffle.description.clone(),
        join_url: join_url(app, &raffle.join_code),
        join_code: raffle.join_code.clone(),
        entrant_count: entrants
            .iter()
            .filter(|e| !e.erased || state != RaffleState::Purged)
            .count(),
        recent_names,
        prizes: prize_views,
        items,
        current,
        draws,
        last_failure,
        manifest_hash: raffle.manifest_hash.clone(),
        server_now_ms: rafi_store::now_ms(),
        test_mode: raffle.test_mode,
        verifiable,
        state,
    })
}
