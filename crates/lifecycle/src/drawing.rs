//! How a raffle uses a drawing: the one place where the raffle's terms (prizes
//! with quantities, its lifecycle) meet the drawing's (items, the manifest, the
//! events). A raffle's drawing has the raffle's ID ([`drawing_id`],
//! [`raffle_id`]).

use rafi_raffle::{
    BeaconSource, Drawing, DrawingId, EntrantId, Event, ItemId, Manifest, ManifestItem, Outcome,
};

use crate::RaffleState;

/// The drawing of raffle `raffle_id`: it has the raffle's ID.
pub fn drawing_id(raffle_id: &str) -> DrawingId {
    DrawingId::new(raffle_id)
}

/// The raffle drawing `drawing` belongs to (the inverse of [`drawing_id`]):
/// what clients following a drawing's changes follow.
pub fn raffle_id(drawing: &DrawingId) -> &str {
    drawing.as_str()
}

/// The drawing's item for the `n`th copy (from 1) of prize `prize_id`.
pub fn item_id(prize_id: &str, n: u64) -> ItemId {
    format!("{prize_id}-{n}").into()
}

/// The prize an item was made from by [`item_id`].
pub fn prize_id(item: &ItemId) -> &str {
    let id = item.as_str();
    id.rsplit_once('-').map_or(id, |(prize_id, _)| prize_id)
}

/// Expands prizes `(prize_id, name, quantity)` into the drawing's items: one
/// per copy, in order.
pub fn items<'a>(prizes: impl IntoIterator<Item = (&'a str, &'a str, u64)>) -> Vec<ManifestItem> {
    prizes
        .into_iter()
        .flat_map(|(prize_id, name, qty)| {
            (1..=qty).map(move |n| ManifestItem {
                id: item_id(prize_id, n),
                name: name.to_owned(),
            })
        })
        .collect()
}

/// The manifest raffle `raffle_id` opens its drawing with: its active
/// entrants and its prizes' items, drawn from `beacon`.
pub fn manifest(
    raffle_id: &str,
    beacon: BeaconSource,
    entrants: impl IntoIterator<Item = String>,
    items: Vec<ManifestItem>,
) -> Manifest {
    Manifest::new(
        drawing_id(raffle_id),
        beacon,
        entrants.into_iter().map(EntrantId::from).collect(),
        items,
    )
}

/// The lifecycle state a raffle is in while its drawing stands so: frozen
/// until a draw starts, then drawing until the drawing finishes.
pub fn lifecycle(drawing: &Drawing) -> RaffleState {
    if drawing.is_finished() {
        RaffleState::Finished
    } else if drawing
        .events()
        .iter()
        .any(|e| matches!(e, Event::Announced { .. } | Event::Drawn { .. }))
    {
        RaffleState::Drawing
    } else {
        RaffleState::Frozen
    }
}

/// Whether `entrant`'s prize can be marked delivered (or not): they won an
/// item and claimed it. Delivery dates when a winner's data may go
/// (retention), so only a winner's counts.
pub fn deliverable(drawing: Option<&Drawing>, entrant: &str) -> bool {
    drawing.is_some_and(|d| {
        d.entrant_outcomes()
            .get(entrant)
            .is_some_and(|(outcome, _)| *outcome == Outcome::Claimed)
    })
}

/// Whether a raffle in `state` with `drawing` can be deleted: not while a
/// draw waits for its beacon (the engine would write to a raffle that's gone).
pub fn deletable(state: RaffleState, drawing: Option<&Drawing>) -> bool {
    !(state == RaffleState::Drawing && drawing.is_some_and(|d| d.pending().is_some()))
}

#[cfg(test)]
mod tests {
    use rafi_raffle::{Decision, DrawRequest};

    use super::*;

    #[test]
    fn prizes_expand_into_items_in_order() {
        let items = items([("a", "Book", 2), ("b", "Mug", 0), ("c", "Pen", 1)]);
        let ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["a-1", "a-2", "c-1"]);
        assert_eq!(items[2].name, "Pen");
    }

    #[test]
    fn an_item_knows_its_prize() {
        assert_eq!(prize_id(&item_id("0f3a", 12)), "0f3a");
        assert_eq!(prize_id(&item_id("with-dash", 1)), "with-dash");
    }

    #[test]
    fn the_raffle_follows_its_drawing() {
        let manifest = manifest(
            "r1",
            BeaconSource::QUICKNET,
            ["aa".to_owned(), "bb".to_owned()],
            items([("book", "Book", 1)]),
        );
        let mut drawing = Drawing::open(&manifest).unwrap();
        assert_eq!(lifecycle(&drawing), RaffleState::Frozen);
        let book = DrawRequest {
            item: "book-1".into(),
        };
        drawing
            .announce(&book, &BeaconSource::QUICKNET, 1, 0)
            .unwrap();
        assert_eq!(lifecycle(&drawing), RaffleState::Drawing);
        drawing
            .decide(&Decision::Abandon {
                item: "book-1".into(),
            })
            .unwrap();
        drawing.decide(&Decision::Finish).unwrap();
        assert_eq!(lifecycle(&drawing), RaffleState::Finished);
    }

    fn opened() -> Drawing {
        let manifest = manifest(
            "r1",
            BeaconSource::QUICKNET,
            ["aa".to_owned()],
            items([("book", "Book", 2)]),
        );
        Drawing::open(&manifest).unwrap()
    }

    #[test]
    fn deletable_unless_a_draw_waits_for_its_beacon() {
        let mut drawing = opened();
        for s in RaffleState::ALL {
            assert!(deletable(s, None), "{s:?} without a drawing");
            assert!(deletable(s, Some(&drawing)), "{s:?} with nothing pending");
        }
        let book = DrawRequest {
            item: "book-1".into(),
        };
        drawing
            .announce(&book, &BeaconSource::QUICKNET, 1, 0)
            .unwrap();
        assert!(!deletable(RaffleState::Drawing, Some(&drawing)));
        assert!(
            deletable(RaffleState::Frozen, Some(&drawing)),
            "only a drawing raffle waits"
        );
    }

    #[test]
    fn only_a_claimed_winner_is_delivered_to() {
        assert!(!deliverable(None, "aa"));
        assert!(!deliverable(Some(&opened()), "aa"), "nobody drawn yet");
    }
}
