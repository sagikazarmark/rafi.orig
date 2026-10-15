//! The participant view of a raffle, built from what the servers read
//! ([`Snapshot`]). The phone only reads views, so it needs neither
//! `rafi-lifecycle` nor `rafi-raffle`.

use rafi_lifecycle::RaffleState;
use rafi_lifecycle::live::Snapshot;
use rafi_raffle::{ItemId, Outcome};

use rafi_live_client::{Current, DrawInput, DrawOutcome, ParticipantView, Phase, public_name};

/// The phase phones see for `state`. One to one: a state added to the
/// raffle doesn't build until phones know it.
pub fn phase(state: RaffleState) -> Phase {
    match state {
        RaffleState::Draft => Phase::Draft,
        RaffleState::Ready => Phase::Ready,
        RaffleState::Open => Phase::Open,
        RaffleState::Closed => Phase::Closed,
        RaffleState::Frozen => Phase::Frozen,
        RaffleState::Drawing => Phase::Drawing,
        RaffleState::Finished => Phase::Finished,
        RaffleState::Purged => Phase::Purged,
    }
}

fn outcome(outcome: Outcome) -> DrawOutcome {
    match outcome {
        Outcome::Drawn => DrawOutcome::Drawn,
        Outcome::Claimed => DrawOutcome::Claimed,
        Outcome::NotHere => DrawOutcome::NotHere,
        Outcome::Declined => DrawOutcome::Declined,
    }
}

/// What phones see of a raffle read as `snapshot`. Every server that shows
/// phones a raffle builds it here: the app server, the polling endpoint and
/// the raffle's live channel.
pub fn view(snapshot: &Snapshot) -> ParticipantView {
    let state = phase(snapshot.state);
    let Some(raffle) = &snapshot.drawn else {
        return ParticipantView::new(snapshot.version, state, None, [], snapshot.prize_qty);
    };
    let item_name = |id: &ItemId| raffle.item(id).map(|i| i.name.clone()).unwrap_or_default();
    let draws = raffle.draws().into_iter().map(|d| {
        let id = d.entrant.as_str();
        let display_name = public_name(id, snapshot.names.get(id).map(Option::as_deref));
        DrawInput {
            display_name,
            prize_name: item_name(&d.item),
            outcome: outcome(d.outcome),
            entrant_id: d.entrant.into_string(),
        }
    });
    let current = raffle.pending().map(|p| Current {
        prize_name: item_name(&p.item),
        round: p.round,
        round_time: p.round_time,
    });
    ParticipantView::new(
        snapshot.version,
        state,
        current,
        draws.collect::<Vec<_>>(),
        raffle.items_left() as u32,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rafi_raffle::{BeaconSource, Event, Manifest, ManifestItem, Raffle, Resolution};

    use rafi_live_client::HIDDEN_NAME;

    use super::*;

    fn manifest() -> Manifest {
        Manifest::new(
            "r1",
            BeaconSource::QUICKNET,
            vec!["a".repeat(32).into(), "b".repeat(32).into()],
            ["u1", "u2"]
                .map(|id| ManifestItem {
                    id: id.into(),
                    name: if id == "u1" { "Book" } else { "Shirt" }.into(),
                })
                .into(),
        )
    }

    fn drawn(k: u64, entrant: &str, item: &str) -> Event {
        Event::Drawn {
            k,
            item: item.into(),
            round: 1,
            signature: String::new(),
            randomness: String::new(),
            pass: 1,
            pool_size: 2,
            index: 0,
            entrant: entrant.into(),
        }
    }

    fn snapshot(drawn: Option<Raffle>, names: HashMap<String, Option<String>>) -> Snapshot {
        Snapshot {
            state: RaffleState::Drawing,
            version: 4,
            live_ended_at: None,
            drawn,
            names,
            prize_qty: 2,
        }
    }

    #[test]
    fn every_state_has_its_phase() {
        for state in RaffleState::ALL {
            let phase = phase(state);
            assert_eq!(
                serde_json::to_value(phase).unwrap(),
                serde_json::to_value(state).unwrap()
            );
            assert_eq!(phase.is_over(), state.is_over());
            assert_eq!(phase.is_live(), state.is_live());
        }
    }

    #[test]
    fn before_the_raffle_opens_it_counts_prizes() {
        let v = view(&snapshot(None, HashMap::new()));
        assert_eq!(v.prizes_left, 2);
        assert_eq!(v.v, 4);
        assert!(v.winners.is_empty());
    }

    #[test]
    fn hidden_names_are_masked_and_no_shows_skipped() {
        let (a, b) = ("a".repeat(32), "b".repeat(32));
        let raffle = Raffle::restore(
            &manifest(),
            &[
                drawn(1, &a, "u1"),
                Event::Resolved {
                    k: 1,
                    resolution: Resolution::Claimed,
                },
                drawn(2, &b, "u2"),
            ],
        )
        .unwrap();
        let names = HashMap::from([(a.clone(), Some("Anna".to_owned())), (b.clone(), None)]);
        let v = view(&snapshot(Some(raffle), names.clone()));
        assert_eq!(v.last.as_ref().unwrap().display_name, HIDDEN_NAME);
        assert_eq!(v.winners.len(), 2);
        assert!(v.winners[0].claimed);
        assert_eq!(v.prizes_left, 1, "a drawn item is open until claimed");

        let raffle = Raffle::restore(
            &manifest(),
            &[
                drawn(1, &a, "u1"),
                Event::Resolved {
                    k: 1,
                    resolution: Resolution::Claimed,
                },
                drawn(2, &b, "u2"),
                Event::Resolved {
                    k: 2,
                    resolution: Resolution::NotHere,
                },
            ],
        )
        .unwrap();
        let v = view(&snapshot(Some(raffle), names));
        assert_eq!(v.winners.len(), 1);
        assert_eq!(
            v.last.unwrap().display_name,
            "Anna",
            "the no-show is skipped"
        );
    }

    #[test]
    fn a_name_not_read_shows_the_id() {
        let raffle = Raffle::restore(&manifest(), &[drawn(1, &"a".repeat(32), "u1")]).unwrap();
        let v = view(&snapshot(Some(raffle), HashMap::new()));
        assert_eq!(v.last.unwrap().display_name, "#aaaaaa");
    }
}
