//! The participant view of a raffle and its drawing (the `raffle` feature):
//! for the servers that build views. The phone only reads them, so it builds
//! without this and without `rafi-drawing`.

use std::collections::HashMap;

use rafi_drawing::{Drawing, ItemId, Outcome};
use rafi_lifecycle::RaffleState;

use crate::{Current, DrawInput, DrawOutcome, ParticipantView, Phase};

impl From<RaffleState> for Phase {
    /// One to one: a state added to the raffle doesn't build until phones
    /// know it.
    fn from(state: RaffleState) -> Self {
        match state {
            RaffleState::Draft => Self::Draft,
            RaffleState::Ready => Self::Ready,
            RaffleState::Open => Self::Open,
            RaffleState::Closed => Self::Closed,
            RaffleState::Frozen => Self::Frozen,
            RaffleState::Drawing => Self::Drawing,
            RaffleState::Finished => Self::Finished,
            RaffleState::Purged => Self::Purged,
        }
    }
}

impl From<Outcome> for DrawOutcome {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Drawn => Self::Drawn,
            Outcome::Claimed => Self::Claimed,
            Outcome::NotHere => Self::NotHere,
            Outcome::Declined => Self::Declined,
        }
    }
}

/// What a participant view is built from: one consistent read of the raffle.
#[derive(Debug, Clone, Copy)]
pub struct Raffle<'a> {
    /// The raffle's version: the view's [`v`](ParticipantView::v).
    pub version: u64,
    pub state: RaffleState,
    /// The drawing, once the engine opened it.
    pub drawing: Option<&'a Drawing>,
    /// The display names of the drawn entrants only, by entrant ID; `None`
    /// for one the organizer hid (shown as [`HIDDEN_NAME`](crate::HIDDEN_NAME)).
    pub names: &'a HashMap<String, Option<String>>,
    /// The prizes' total quantity: what is left before the drawing opens.
    pub prize_qty: u32,
}

/// The participant view of `raffle`.
pub fn view(raffle: Raffle<'_>) -> ParticipantView {
    let state = raffle.state.into();
    let Some(drawing) = raffle.drawing else {
        return ParticipantView::new(raffle.version, state, None, [], raffle.prize_qty);
    };
    let item_name = |id: &ItemId| drawing.item(id).map(|i| i.name.clone()).unwrap_or_default();
    let draws = drawing.draws().into_iter().map(|d| {
        let id = d.entrant.as_str();
        let display_name = crate::public_name(id, raffle.names.get(id).map(Option::as_deref));
        DrawInput {
            display_name,
            prize_name: item_name(&d.item),
            outcome: d.outcome.into(),
            entrant_id: d.entrant.into_string(),
        }
    });
    let current = drawing.pending().map(|p| Current {
        prize_name: item_name(&p.item),
        round: p.round,
        round_time: p.round_time,
    });
    ParticipantView::new(
        raffle.version,
        state,
        current,
        draws.collect::<Vec<_>>(),
        drawing.items_left() as u32,
    )
}

#[cfg(test)]
mod tests {
    use rafi_drawing::{BeaconSource, Event, Manifest, ManifestItem, Resolution};

    use super::*;
    use crate::HIDDEN_NAME;

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

    fn raffle<'a>(
        drawing: Option<&'a Drawing>,
        names: &'a HashMap<String, Option<String>>,
    ) -> Raffle<'a> {
        Raffle {
            version: 4,
            state: RaffleState::Drawing,
            drawing,
            names,
            prize_qty: 2,
        }
    }

    #[test]
    fn every_state_has_its_phase() {
        for state in RaffleState::ALL {
            let phase = Phase::from(state);
            assert_eq!(
                serde_json::to_value(phase).unwrap(),
                serde_json::to_value(state).unwrap()
            );
            assert_eq!(phase.is_over(), state.is_over());
            assert_eq!(phase.is_live(), state.is_live());
        }
    }

    #[test]
    fn before_the_drawing_counts_prizes() {
        let names = HashMap::new();
        let v = view(raffle(None, &names));
        assert_eq!(v.prizes_left, 2);
        assert_eq!(v.v, 4);
        assert!(v.winners.is_empty());
    }

    #[test]
    fn hidden_names_are_masked_and_no_shows_skipped() {
        let (a, b) = ("a".repeat(32), "b".repeat(32));
        let drawing = Drawing::restore(
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
        let v = view(raffle(Some(&drawing), &names));
        assert_eq!(v.last.as_ref().unwrap().display_name, HIDDEN_NAME);
        assert_eq!(v.winners.len(), 2);
        assert!(v.winners[0].claimed);
        assert_eq!(v.prizes_left, 1, "a drawn item is open until claimed");

        let drawing = Drawing::restore(
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
        let v = view(raffle(Some(&drawing), &names));
        assert_eq!(v.winners.len(), 1);
        assert_eq!(
            v.last.unwrap().display_name,
            "Anna",
            "the no-show is skipped"
        );
    }

    #[test]
    fn a_name_not_read_shows_the_id() {
        let drawing = Drawing::restore(&manifest(), &[drawn(1, &"a".repeat(32), "u1")]).unwrap();
        let v = view(raffle(Some(&drawing), &HashMap::new()));
        assert_eq!(v.last.unwrap().display_name, "#aaaaaa");
    }
}
