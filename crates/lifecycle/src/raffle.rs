//! Where the lifecycle meets the raffle: the one place where the app's terms
//! (prizes with quantities, the lifecycle) meet the raffle's (items, the
//! manifest, the events). Both name a raffle by the same ID.

use rafi_raffle::{
    BeaconSource, EntrantId, Event, ItemId, Manifest, ManifestItem, Outcome, Raffle, RaffleId,
};

use crate::{
    RaffleState,
    prize::{Delivery, OnClaim},
};

/// The raffle's item for the `n`th copy (from 1) of prize `prize_id`.
pub fn item_id(prize_id: &str, n: u64) -> ItemId {
    format!("{prize_id}-{n}").into()
}

/// The prize an item was made from by [`item_id`].
pub fn prize_id(item: &ItemId) -> &str {
    let id = item.as_str();
    id.rsplit_once('-').map_or(id, |(prize_id, _)| prize_id)
}

/// Expands prizes `(prize_id, name, quantity)` into the raffle's items: one
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

/// The manifest raffle `raffle_id` opens with: its active entrants and its
/// prizes' items, drawn from `beacon`.
pub fn manifest(
    raffle_id: &str,
    beacon: BeaconSource,
    entrants: impl IntoIterator<Item = String>,
    items: Vec<ManifestItem>,
) -> Manifest {
    Manifest::new(
        RaffleId::new(raffle_id),
        beacon,
        entrants.into_iter().map(EntrantId::from).collect(),
        items,
    )
}

/// The lifecycle state of `raffle` as drawn so far: frozen until a draw
/// starts, then drawing until it finishes.
pub fn lifecycle(raffle: &Raffle) -> RaffleState {
    if raffle.is_finished() {
        RaffleState::Finished
    } else if raffle
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
/// item and claimed it. Only a winner's prize is delivered, so only a
/// winner's counts.
pub fn deliverable(raffle: Option<&Raffle>, entrant: &str) -> bool {
    raffle.is_some_and(|d| {
        d.entrant_outcomes()
            .get(entrant)
            .is_some_and(|(outcome, _)| *outcome == Outcome::Claimed)
    })
}

/// What claiming draw `k` of `raffle` does to its winner's delivery, its
/// prize delivered as `delivery` says now (by prize ID; `None` for one that
/// isn't known). The draw may be claimed already (the store, projecting the
/// claim) or not yet (the app, sending it). Its winner's other claims count:
/// they are delivered by this claim only if every prize they claimed is
/// handed over on the spot. Each of those is taken as it was delivered when
/// it was claimed (`claimed_as`, by draw; the prize's delivery now for one
/// not recorded yet): a prize switched to "on the spot" after its claim is
/// still owed to whoever claimed it as "sent later".
pub fn on_claim(
    raffle: &Raffle,
    k: u64,
    delivery: impl Fn(&str) -> Option<Delivery>,
    claimed_as: impl Fn(u64) -> Option<Delivery>,
) -> OnClaim {
    let draws = raffle.draws();
    let Some(claim) = draws.iter().find(|d| d.k == k) else {
        return OnClaim::Nothing;
    };
    let of = |item: &ItemId| delivery(prize_id(item));
    match of(&claim.item) {
        None => OnClaim::Nothing,
        Some(Delivery::Later) => OnClaim::Undeliver,
        Some(Delivery::OnTheSpot) => {
            let all_on_the_spot = draws
                .iter()
                .filter(|d| d.k != k && d.entrant == claim.entrant && d.outcome == Outcome::Claimed)
                .all(|d| claimed_as(d.k).or_else(|| of(&d.item)) == Some(Delivery::OnTheSpot));
            if all_on_the_spot {
                OnClaim::Deliver
            } else {
                OnClaim::Nothing
            }
        }
    }
}

/// Whether a raffle drawn as `raffle` can be deleted: until its first draw
/// is announced. From then on its record (the manifest hash on the
/// verification page, the event log with every draw and abandon) is what
/// makes the draw verifiable, and deleting it would let an organizer draw,
/// dislike the winner, and start over with no trace; "Erase personal data
/// now" is the way to be rid of it. A frozen raffle with no draw stays
/// deletable: it is the way out of a freeze by accident (organizers can't
/// unfreeze), and deleting it takes its entrants with it, so a "fixed" list
/// means the whole room joins again under a new code.
pub fn deletable(raffle: Option<&Raffle>) -> bool {
    !raffle.is_some_and(|d| {
        d.events()
            .iter()
            .any(|e| matches!(e, Event::Announced { .. }))
    })
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
    fn the_lifecycle_follows_the_raffle() {
        let manifest = manifest(
            "r1",
            BeaconSource::QUICKNET,
            ["aa".to_owned(), "bb".to_owned()],
            items([("book", "Book", 1)]),
        );
        let mut raffle = Raffle::open(&manifest).unwrap();
        assert_eq!(lifecycle(&raffle), RaffleState::Frozen);
        let book = DrawRequest {
            item: "book-1".into(),
        };
        raffle
            .announce(&book, &BeaconSource::QUICKNET, 1, 0)
            .unwrap();
        assert_eq!(lifecycle(&raffle), RaffleState::Drawing);
        raffle
            .decide(&Decision::Abandon {
                item: "book-1".into(),
            })
            .unwrap();
        raffle.decide(&Decision::Finish).unwrap();
        assert_eq!(lifecycle(&raffle), RaffleState::Finished);
    }

    /// One entrant, Anna, and more items than people: she claims a book
    /// handed over on the spot, then in pass 2 a voucher sent later, then a
    /// mug handed over.
    #[test]
    fn a_claim_delivers_when_every_prize_claimed_was_handed_over() {
        use rafi_raffle::Resolution;

        let anna = "aa".to_owned();
        let manifest = manifest(
            "r1",
            BeaconSource::QUICKNET,
            [anna.clone()],
            items([
                ("book", "Book", 1),
                ("voucher", "Voucher", 1),
                ("mug", "Mug", 1),
            ]),
        );
        let drawn = |k: u64, item: &str| Event::Drawn {
            k,
            item: item.into(),
            round: k,
            signature: String::new(),
            randomness: String::new(),
            pass: if k == 1 { 1 } else { 2 },
            pool_size: 1,
            index: 0,
            entrant: anna.clone().into(),
        };
        let claimed = |k| Event::Resolved {
            k,
            resolution: Resolution::Claimed,
        };
        let delivery = |prize: &str| match prize {
            "book" | "mug" => Some(Delivery::OnTheSpot),
            "voucher" => Some(Delivery::Later),
            _ => None,
        };
        let at = |events: &[Event]| Raffle::restore(&manifest, events).unwrap();
        // No claim recorded with its delivery yet: each prize as it is now.
        let none = |_: u64| None;

        // The app asks before the claim, the store after it: the same answer.
        let events = [drawn(1, "book-1")];
        assert_eq!(on_claim(&at(&events), 1, delivery, none), OnClaim::Deliver);
        let events = [drawn(1, "book-1"), claimed(1)];
        assert_eq!(on_claim(&at(&events), 1, delivery, none), OnClaim::Deliver);

        let events = [
            drawn(1, "book-1"),
            claimed(1),
            drawn(2, "voucher-1"),
            claimed(2),
        ];
        assert_eq!(
            on_claim(&at(&events), 2, delivery, none),
            OnClaim::Undeliver,
            "she is owed the voucher now"
        );
        let events = [
            drawn(1, "book-1"),
            claimed(1),
            drawn(2, "voucher-1"),
            claimed(2),
            drawn(3, "mug-1"),
            claimed(3),
        ];
        assert_eq!(
            on_claim(&at(&events), 3, delivery, none),
            OnClaim::Nothing,
            "the mug is handed over, the voucher still isn't"
        );
        assert_eq!(
            on_claim(&at(&events), 3, |_| None, none),
            OnClaim::Nothing,
            "unknown prizes"
        );
        assert_eq!(
            on_claim(&at(&events), 9, delivery, none),
            OnClaim::Nothing,
            "no such draw"
        );

        // The voucher was claimed as sent later; the organizer then switches
        // it to on the spot (for the copies still to come). The mug's claim
        // doesn't deliver her: the voucher she claimed is still owed.
        let switched = |_: &str| Some(Delivery::OnTheSpot);
        let as_claimed = |k: u64| match k {
            1 => Some(Delivery::OnTheSpot),
            2 => Some(Delivery::Later),
            _ => None,
        };
        assert_eq!(
            on_claim(&at(&events), 3, switched, as_claimed),
            OnClaim::Nothing,
            "an earlier claim keeps the delivery it was claimed with"
        );
        assert_eq!(
            on_claim(&at(&events), 3, switched, none),
            OnClaim::Deliver,
            "unrecorded, it is taken as the prize is now"
        );
    }

    fn opened() -> Raffle {
        let manifest = manifest(
            "r1",
            BeaconSource::QUICKNET,
            ["aa".to_owned()],
            items([("book", "Book", 2)]),
        );
        Raffle::open(&manifest).unwrap()
    }

    #[test]
    fn deletable_until_the_first_draw_is_announced() {
        let mut raffle = opened();
        assert!(deletable(None), "never opened in the engine");
        assert!(deletable(Some(&raffle)), "frozen, nothing drawn");
        let book = DrawRequest {
            item: "book-1".into(),
        };
        raffle
            .announce(&book, &BeaconSource::QUICKNET, 1, 0)
            .unwrap();
        assert!(!deletable(Some(&raffle)), "a draw is pending");
        raffle
            .decide(&Decision::Abandon {
                item: "book-1".into(),
            })
            .unwrap();
        assert!(
            !deletable(Some(&raffle)),
            "an abandoned draw is part of the record too"
        );
        let finished = {
            let mut r = opened();
            r.decide(&Decision::Finish).unwrap();
            r
        };
        assert!(
            deletable(Some(&finished)),
            "finished without a draw: nothing to keep verifiable"
        );
    }

    #[test]
    fn only_a_claimed_winner_is_delivered_to() {
        assert!(!deliverable(None, "aa"));
        assert!(!deliverable(Some(&opened()), "aa"), "nobody drawn yet");
    }
}
