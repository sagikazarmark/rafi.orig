//! The participant view of a raffle, built from what the servers read. The
//! phone only reads views, so it needs neither `rafi-lifecycle` nor
//! `rafi-raffle`.

use std::collections::HashMap;

use rafi_lifecycle::RaffleState;
use rafi_raffle::{ItemId, Outcome, Raffle};

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

/// What a participant view is built from: one consistent read of the raffle.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The raffle's version: the view's [`v`](ParticipantView::v).
    pub version: u64,
    pub state: RaffleState,
    /// The raffle as drawn, once the engine opened it.
    pub raffle: Option<&'a Raffle>,
    /// The display names of the drawn entrants only, by entrant ID; `None`
    /// for one the organizer hid (shown as [`HIDDEN_NAME`](rafi_live_client::HIDDEN_NAME)).
    pub names: &'a HashMap<String, Option<String>>,
    /// The prizes' total quantity: what is left before the raffle opens.
    pub prize_qty: u32,
}

/// The participant view of a raffle.
pub fn view(input: Input<'_>) -> ParticipantView {
    let state = phase(input.state);
    let Some(raffle) = input.raffle else {
        return ParticipantView::new(input.version, state, None, [], input.prize_qty);
    };
    let item_name = |id: &ItemId| raffle.item(id).map(|i| i.name.clone()).unwrap_or_default();
    let draws = raffle.draws().into_iter().map(|d| {
        let id = d.entrant.as_str();
        let display_name = public_name(id, input.names.get(id).map(Option::as_deref));
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
        input.version,
        state,
        current,
        draws.collect::<Vec<_>>(),
        raffle.items_left() as u32,
    )
}

#[cfg(test)]
mod tests {
    use rafi_raffle::{BeaconSource, Event, Manifest, ManifestItem, Resolution};

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

    fn input<'a>(
        raffle: Option<&'a Raffle>,
        names: &'a HashMap<String, Option<String>>,
    ) -> Input<'a> {
        Input {
            version: 4,
            state: RaffleState::Drawing,
            raffle,
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
        let names = HashMap::new();
        let v = view(input(None, &names));
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
        let v = view(input(Some(&raffle), &names));
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
        let v = view(input(Some(&raffle), &names));
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
        let v = view(input(Some(&raffle), &HashMap::new()));
        assert_eq!(v.last.unwrap().display_name, "#aaaaaa");
    }
}
